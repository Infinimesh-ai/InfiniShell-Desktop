#!/usr/bin/env python3
"""真实验收运行器必须同时取得测试匹配、退出成功及对应协议终态。"""

import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
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
    run,
    verify_candidate_01561_package,
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

    def test_candidate_01561_requires_fixed_complete_package_before_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="codex-candidate-test-") as temporary:
            root = Path(temporary).resolve(strict=True) / "package"
            (root / "bin").mkdir(parents=True)
            executable = root / "bin/codex"
            executable.write_bytes(b"fixture")
            executable = executable.resolve(strict=True)
            root = executable.parent.parent
            self.assertEqual(executable.resolve(strict=True), executable)
            package = {"entrypoint": "bin/codex"}
            with mock.patch("run_codex_adapter_live.sys.platform", "darwin"), \
                    mock.patch("run_codex_adapter_live.platform.machine", return_value="arm64"), \
                    mock.patch("run_codex_adapter_live.prepare.packages_for_version",
                               return_value={"macos-arm64": package}) as manifest, \
                    mock.patch("run_codex_adapter_live.prepare.verify_runtime_tree",
                               return_value=executable) as verified:
                self.assertEqual(verify_candidate_01561_package(executable), "macos-arm64")
                manifest.assert_called_once_with("0.156.1")
                verified.assert_called_once_with(root, package, "0.156.1")
                with self.assertRaisesRegex(ValueError, "固定完整包的入口"):
                    verify_candidate_01561_package(root / "bin/not-codex")
            output = Path(temporary) / "receipt.ndjson"
            for case in ("candidate-01561-missing-session", "candidate-01561-lifecycle",
                         "candidate-01561-running-tool-cancel"):
                with self.subTest(case=case):
                    args = SimpleNamespace(test_case=case, codex=executable, output=output)
                    with mock.patch("run_codex_adapter_live.verify_candidate_01561_package",
                                    side_effect=ValueError("fixed package rejected")), \
                            mock.patch("run_codex_adapter_live.subprocess.run") as process:
                        with self.assertRaisesRegex(ValueError, "fixed package rejected"):
                            run(args)
                        process.assert_not_called()
                    self.assertFalse(output.exists())

    def test_candidate_01561_windows_package_entrypoint_supports_private_temp_path(self):
        with tempfile.TemporaryDirectory(prefix="runner 临时 & ") as temporary:
            root = Path(temporary).resolve(strict=True) / "codex package"
            (root / "bin").mkdir(parents=True)
            executable = root / "bin/codex.exe"
            executable.write_bytes(b"fixture")
            executable = executable.resolve(strict=True)
            root = executable.parent.parent
            self.assertEqual(executable.resolve(strict=True), executable)
            package = {"entrypoint": "bin/codex.exe"}
            for machine, target in (("AMD64", "windows-x64"), ("ARM64", "windows-arm64")):
                with self.subTest(machine=machine), \
                        mock.patch("run_codex_adapter_live.sys.platform", "win32"), \
                        mock.patch("run_codex_adapter_live.platform.machine", return_value=machine), \
                        mock.patch("run_codex_adapter_live.prepare.packages_for_version",
                                   return_value={target: package}), \
                        mock.patch("run_codex_adapter_live.prepare.verify_runtime_tree",
                                   return_value=executable) as verified:
                    self.assertEqual(verify_candidate_01561_package(executable), target)
                    verified.assert_called_once_with(root, package, "0.156.1")

    def test_candidate_01561_rejects_noncanonical_entrypoint(self):
        with tempfile.TemporaryDirectory(prefix="codex-entrypoint-test-") as temporary:
            root = Path(temporary).resolve(strict=True) / "package"
            (root / "bin").mkdir(parents=True)
            executable = root / "bin/codex"
            executable.write_bytes(b"fixture")
            noncanonical = root / "bin/../bin/codex"
            self.assertTrue(noncanonical.is_file())
            self.assertNotEqual(noncanonical.resolve(strict=True), noncanonical)
            with mock.patch("run_codex_adapter_live.sys.platform", "darwin"), \
                    mock.patch("run_codex_adapter_live.platform.machine", return_value="arm64"), \
                    mock.patch("run_codex_adapter_live.prepare.packages_for_version",
                               return_value={"macos-arm64": {"entrypoint": "bin/codex"}}), \
                    mock.patch("run_codex_adapter_live.prepare.verify_runtime_tree") as verified:
                with self.assertRaisesRegex(ValueError, "固定完整包的入口"):
                    verify_candidate_01561_package(noncanonical)
                verified.assert_not_called()

    def test_candidate_01561_rejects_symlink_entrypoint(self):
        with tempfile.TemporaryDirectory(prefix="codex-entrypoint-test-") as temporary:
            root = Path(temporary).resolve(strict=True) / "package"
            (root / "bin").mkdir(parents=True)
            native = root / "bin/native"
            native.write_bytes(b"fixture")
            link = root / "bin/codex"
            try:
                link.symlink_to(native)
            except (OSError, NotImplementedError) as error:
                self.skipTest(f"当前系统不能创建测试符号链接：{type(error).__name__}")
            self.assertTrue(link.is_file())
            self.assertNotEqual(link.resolve(strict=True), link)
            with mock.patch("run_codex_adapter_live.sys.platform", "darwin"), \
                    mock.patch("run_codex_adapter_live.platform.machine", return_value="arm64"), \
                    mock.patch("run_codex_adapter_live.prepare.packages_for_version",
                               return_value={"macos-arm64": {"entrypoint": "bin/codex"}}), \
                    mock.patch("run_codex_adapter_live.prepare.verify_runtime_tree") as verified:
                with self.assertRaisesRegex(ValueError, "固定完整包的入口"):
                    verify_candidate_01561_package(link)
                verified.assert_not_called()

    def test_candidate_01561_lifecycle_uses_runner_temp_before_credential_copy(self):
        with tempfile.TemporaryDirectory(prefix="runner 临时 & ") as temporary:
            root = Path(temporary)
            output = root / "receipt.ndjson"
            args = SimpleNamespace(test_case="candidate-01561-lifecycle",
                                   codex=root / "package/bin/codex.exe",
                                   test_binary=root / "warp-tests.exe",
                                   supervisor=root / "warp.exe", output=output)
            with mock.patch.dict(os.environ, {"RUNNER_TEMP": temporary}), \
                    mock.patch("run_codex_adapter_live.sys.platform", "win32"), \
                    mock.patch("run_codex_adapter_live.verify_candidate_01561_package"), \
                    mock.patch("run_codex_adapter_live.digest", return_value="fixture"), \
                    mock.patch("run_codex_adapter_live.subprocess.run",
                               return_value=mock.Mock(stdout="fixture\n")), \
                    mock.patch("run_codex_adapter_live.tempfile.TemporaryDirectory",
                               side_effect=RuntimeError("stop before credential copy")) as directory:
                with self.assertRaisesRegex(RuntimeError, "stop before credential copy"):
                    run(args)
            directory.assert_called_once_with(prefix="infinishell-codex-adapter-", dir=temporary)
            self.assertFalse(output.exists())

    def test_each_case_requires_current_success_and_matching_test(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        zero = "test result: ok. 0 passed; 0 failed; 0 ignored;"
        cases = {
            "lifecycle": {"event": "acceptance_passed"},
            "candidate-01561-lifecycle": {
                "event": "acceptance_passed", "scope": "rust_adapter_01561_test_only_process_restart",
                "test_only_candidate_01561": True, "app_restart_and_ui_verified": False,
            },
            "running-tool-cancel": {
                "event": "running_tool_cancel_finished", "passed": True,
                "scope": "rust_adapter_running_tool_cancel", "test_only_candidate_01561": False,
                "native_item_started_before_interrupt": True, "interrupt_native_ack": True,
                "same_generation_receipt": True, "cleanup_confirmed": True,
                "tool_tree_zero_residual": True, "terminal_before_disconnected": True,
                "event_order": ["native_item_started", "interrupt_sent", "interrupt_accepted",
                                "turn_finished", "disconnected"],
                "containment": "macos_resource_coalition",
            },
            "candidate-01561-running-tool-cancel": {
                "event": "running_tool_cancel_finished", "passed": True,
                "scope": "rust_adapter_01561_test_only_running_tool_cancel",
                "test_only_candidate_01561": True,
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
            "candidate-01561-missing-session": {
                "event": "missing_session_probe_finished", "passed": True,
                "test_only_candidate_01561": True, "credentials_provided": False,
                "model_commands_sent": 0,
            },
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
        self.assertFalse(verified_acceptance("candidate-01561-missing-session", 0, summary,
                                             [{"event": "missing_session_probe_finished", "passed": True}]))
        for event in (
            {"event": "acceptance_passed"},
            {"event": "acceptance_passed", "scope": "rust_adapter_process_restart",
             "test_only_candidate_01561": True, "app_restart_and_ui_verified": False},
            {"event": "acceptance_passed", "scope": "rust_adapter_01561_test_only_process_restart",
             "test_only_candidate_01561": False, "app_restart_and_ui_verified": False},
        ):
            self.assertFalse(verified_acceptance("candidate-01561-lifecycle", 0, summary, [event]))

    def test_running_tool_cancel_rejects_missing_cleanup_or_wrong_order(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        valid = {
            "event": "running_tool_cancel_finished", "passed": True,
            "scope": "rust_adapter_running_tool_cancel", "test_only_candidate_01561": False,
            "native_item_started_before_interrupt": True, "interrupt_native_ack": True,
            "same_generation_receipt": True, "cleanup_confirmed": True,
            "tool_tree_zero_residual": True, "terminal_before_disconnected": True,
            "event_order": ["native_item_started", "interrupt_sent", "interrupt_accepted",
                            "turn_finished", "disconnected"],
            "containment": "macos_resource_coalition",
        }
        for case, scope, candidate in (
            ("running-tool-cancel", "rust_adapter_running_tool_cancel", False),
            ("candidate-01561-running-tool-cancel", "rust_adapter_01561_test_only_running_tool_cancel", True),
        ):
            event = valid | {"scope": scope, "test_only_candidate_01561": candidate}
            with self.subTest(case=case):
                self.assertTrue(verified_acceptance(case, 0, summary, [event]))
                for key in ("native_item_started_before_interrupt", "interrupt_native_ack",
                            "same_generation_receipt", "cleanup_confirmed", "tool_tree_zero_residual",
                            "terminal_before_disconnected"):
                    with self.subTest(key=key):
                        self.assertFalse(verified_acceptance(case, 0, summary,
                                                             [event | {key: False}]))
                self.assertFalse(verified_acceptance(case, 0, summary,
                                                     [event | {"event_order": list(reversed(event["event_order"]))}]))
                self.assertFalse(verified_acceptance(case, 0, summary,
                                                     [event | {"containment": "unix_process_group"}]))
                self.assertFalse(verified_acceptance(case, 0, summary,
                                                     [event | {"test_only_candidate_01561": not candidate}]))
                self.assertFalse(verified_acceptance(case, 0, summary,
                                                     [event | {"scope": "other"}]))

    def test_candidate_01561_running_tool_cancel_keeps_outer_cleanup_gate(self):
        with tempfile.TemporaryDirectory(prefix="codex-cancel-runner-") as temporary:
            root = Path(temporary)
            for name in ("codex", "warp-tests", "warp"):
                (root / name).write_bytes(b"fixture")
            credentials = root / "fixture-auth.json"
            credentials.write_bytes(b"{}")
            output = root / "receipt.ndjson"
            args = SimpleNamespace(test_case="candidate-01561-running-tool-cancel",
                                   codex=root / "codex", test_binary=root / "warp-tests",
                                   supervisor=root / "warp", credential_source=credentials,
                                   output=output)
            event = {
                "event": "running_tool_cancel_finished", "passed": True,
                "scope": "rust_adapter_01561_test_only_running_tool_cancel",
                "test_only_candidate_01561": True,
                "native_item_started_before_interrupt": True, "interrupt_native_ack": True,
                "same_generation_receipt": True, "cleanup_confirmed": True,
                "tool_tree_zero_residual": True, "terminal_before_disconnected": True,
                "event_order": ["native_item_started", "interrupt_sent", "interrupt_accepted",
                                "turn_finished", "disconnected"],
                "containment": "macos_resource_coalition",
            }
            process = mock.Mock(returncode=0)
            process.communicate.return_value = ("test result: ok. 1 passed; 0 failed; 0 ignored;", None)
            process.poll.return_value = 0

            def start_test(*_args, **kwargs):
                self.assertEqual(kwargs["env"]["INFINISHELL_CODEX_RUNNING_TOOL_CANCEL"], "1")
                self.assertEqual(kwargs["env"]["INFINISHELL_CODEX_TEST_CANDIDATE_01561"], "1")
                output.write_text(json.dumps(event) + "\n", encoding="utf-8")
                return process

            audit = {"zero_residual": True, "fallback_attempted": False,
                     "unverified_native_root": False, "audit_error": False}
            with mock.patch("run_codex_adapter_live.verify_candidate_01561_package") as verified, \
                    mock.patch("run_codex_adapter_live.subprocess.run",
                               side_effect=[mock.Mock(stdout="a25701d2\n"),
                                            mock.Mock(stdout=""),
                                            mock.Mock(stdout="a25701d2\n"),
                                            mock.Mock(stdout="")]), \
                    mock.patch("run_codex_adapter_live.probe_version",
                               return_value=mock.Mock(stdout="codex-cli 0.156.1\n")), \
                    mock.patch("run_codex_adapter_live.subprocess.Popen", side_effect=start_test), \
                    mock.patch("run_codex_adapter_live.audit_and_cleanup_fixture",
                               side_effect=[audit, audit | {"zero_residual": False}]) as cleanup, \
                    mock.patch("builtins.print"):
                self.assertEqual(run(args), 0)
                self.assertEqual(run(args), 1)
            self.assertEqual(verified.call_count, 2)
            self.assertEqual(cleanup.call_count, 2)

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
