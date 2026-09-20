"""生产安装器验收运行器的离线门禁；模拟证据不计作原生迁移通过。"""

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run_codex_source_live as runner


def complete_events(host_platform="linux"):
    return [
        {"event": "rust_source_migration_started", "scope": "production_rust_installer",
         "credentials_provided": False, "model_commands_sent": 0, "schema_version": 2,
         "platform": {"linux": "linux", "win32": "windows", "darwin": "macos"}[host_platform]},
        {"event": "production_runtime_probe", "succeeded": True},
        {"event": "rev3_fixture_verified", "source_files": 36, "cache_files": 10,
         "initial_installation": "controlled_exact_rev3_fixture", "trust_fixture_is_not_native_authorization": True},
        {"event": "install_observed", "phase": "rev3_to_rev4", "succeeded": True, "native_command_invoked": True},
        {"event": "migration_verified", "rev4_source_and_cache_verified": True,
         "old_source_and_metadata_preserved": True, "old_cache_and_transaction_preserved": True,
         "user_configuration_and_trust_preserved": True, "disabled_orchestration_preserved": True,
         "transaction_phase": "verified"},
        {"event": "install_observed", "phase": "repeat_rev4_install", "succeeded": True, "native_command_invoked": False},
        {"event": "repeat_install_verified", "previous_recovery_material_preserved": True},
        *[{"event": "rejection_observed", "case": case, "rejected": True,
           "bytes_and_modes_unchanged": True, "native_command_invoked": False, "rev4_source_created": False}
          for case in ("modified_rev3_source", "mixed_cache", "unknown_source", "disabled_target")],
        {"event": "rust_source_migration_passed", "passed": True, "model_commands_sent": 0,
         "hook_authorization_performed": False, "native_hook_execution_verified": False,
         "windows_product_gate_opened": host_platform == "win32", "gui_verified": False, "rejected_cases": 4},
    ]


class SourceRunnerTests(unittest.TestCase):
    output = "test result: ok. 1 passed; 0 failed; 0 ignored; 7310 filtered out\n"

    def test_acceptance_requires_exact_test_and_complete_evidence(self):
        self.assertTrue(runner.verified_acceptance(0, self.output, complete_events(), "linux"))
        for code, output in ((1, self.output), (0, "test result: ok. 0 passed; 0 failed; 1 ignored;")):
            self.assertFalse(runner.verified_acceptance(code, output, complete_events(), "linux"))
        for index in range(len(complete_events())):
            events = complete_events()
            events.pop(index)
            self.assertFalse(runner.verified_acceptance(0, self.output, events, "linux"), index)

    def test_rejections_require_all_distinct_cases_and_unchanged_bytes(self):
        for key, value in (("case", "modified_rev3_source"), ("rejected", False),
                           ("bytes_and_modes_unchanged", False), ("native_command_invoked", True),
                           ("rev4_source_created", True)):
            events = complete_events()
            events[8][key] = value
            self.assertFalse(runner.verified_acceptance(0, self.output, events, "linux"), key)

    def test_fake_trust_or_partial_retention_cannot_be_promoted(self):
        for index, key, value in ((2, "trust_fixture_is_not_native_authorization", False),
                                   (2, "source_files", 35), (4, "old_source_and_metadata_preserved", False),
                                   (4, "disabled_orchestration_preserved", False),
                                   (4, "user_configuration_and_trust_preserved", False),
                                   (6, "previous_recovery_material_preserved", False),
                                   (11, "hook_authorization_performed", True),
                                   (11, "windows_product_gate_opened", True), (11, "gui_verified", True),
                                   (11, "native_hook_execution_verified", True), (11, "model_commands_sent", 1)):
            events = complete_events()
            events[index][key] = value
            self.assertFalse(runner.verified_acceptance(0, self.output, events, "linux"), key)

    def test_duplicate_old_or_reordered_records_are_rejected(self):
        events = complete_events()
        self.assertFalse(runner.verified_acceptance(0, self.output, events + copy.deepcopy(events), "linux"))
        events[3], events[5] = events[5], events[3]
        self.assertFalse(runner.verified_acceptance(0, self.output, events, "linux"))

    def test_private_environment_drops_credentials_and_shell_injection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            inherited = {"PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8", "OPENAI_API_KEY": "do-not-inherit",
                         "ANTHROPIC_AUTH_TOKEN": "do-not-inherit", "BASH_ENV": "/user/startup",
                         "HOME": "/user/home", "CODEX_HOME": "/user/codex", "DYLD_INSERT_LIBRARIES": "injected"}
            with patch.dict(os.environ, inherited, clear=True), patch.object(sys, "platform", "linux"):
                environment = runner.isolated_environment(root, root / "fixed/native/codex")
            self.assertEqual(environment["PATH"], str(root / "fixed/native") + os.pathsep + "/usr/bin:/bin")
            self.assertEqual(environment["HOME"], str(root / "home"))
            self.assertEqual(environment["CODEX_HOME"], str(root / "codex"))
            for key in ("OPENAI_API_KEY", "ANTHROPIC_AUTH_TOKEN", "BASH_ENV", "DYLD_INSERT_LIBRARIES"):
                self.assertNotIn(key, environment)
            self.assertEqual((root / ".infinishell-codex-source-probe").read_text(), runner.MARKER)
            self.assertEqual((root / "codex/config.toml").read_text(), 'cli_auth_credentials_store = "file"\n')
            self.assertFalse((root / "codex/auth.json").exists())

    def test_windows_unverified_architecture_remains_rejected_before_input_access(self):
        with patch.object(sys, "platform", "win32"), patch.object(runner.platform, "machine", return_value="ARM64"):
            with self.assertRaisesRegex(ValueError, "x64"):
                runner.validate_inputs(SimpleNamespace(), Path("/repo"))

    def test_existing_evidence_and_input_output_overlap_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary, codex = root / "warp-tests", root / "codex"
            for path in (binary, codex):
                path.write_text("fixture")
                path.chmod(0o700)
            args = SimpleNamespace(test_binary=binary, codex=codex, timeout=300, output=root / "events.ndjson")
            with patch.object(sys, "platform", "linux"):
                runner.validate_inputs(args, root / "repo")
                args.output.write_text("old successful evidence")
                with self.assertRaisesRegex(ValueError, "已有证据"):
                    runner.validate_inputs(args, root / "repo")
                args.output = codex
                with self.assertRaisesRegex(ValueError, "输入文件之外"):
                    runner.validate_inputs(args, root / "repo")
                args.output = root / "repo/events.ndjson"
                with self.assertRaisesRegex(ValueError, "仓库"):
                    runner.validate_inputs(args, root / "repo")

    def test_failed_probe_preserves_private_root_and_cleanup_failure_is_visible(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "private"
            root.mkdir()
            (root / "diagnostic").write_text("keep")
            metadata = {}
            self.assertFalse(runner.finish_private_root(root, False, metadata))
            self.assertTrue((root / "diagnostic").exists())
            with patch.object(runner.shutil, "rmtree", side_effect=PermissionError("original cleanup failure")):
                self.assertFalse(runner.finish_private_root(root, True, metadata))
            self.assertEqual(metadata["cleanup_error"], "original cleanup failure")
            self.assertFalse(metadata["private_root_removed"])
            self.assertTrue(runner.finish_private_root(root, True, metadata))
            self.assertFalse(root.exists())

    @unittest.skipUnless(os.name == "posix", "此用例核验 Unix 进程组；Windows 收尾另有平台回归")
    def test_timeout_retains_partial_output_after_process_group_stop(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "project").mkdir()
            # 系统 shell 不依赖父 Python 的动态库环境；后台子进程同时持有输出管道。
            code, output, timed_out = runner.run_command(
                ["/bin/sh", "-c", "printf 'partial diagnostic\\n'; sleep 30 & wait"],
                root, {"PATH": os.defpath}, 0.2)
            self.assertTrue(timed_out, f"夹具未进入超时：code={code}, output={output!r}")
            self.assertNotEqual(code, 0)
            self.assertIn("partial diagnostic", output)

    @unittest.skipUnless(os.name == "posix", "此用例核验 Unix 进程组；Windows 收尾另有平台回归")
    def test_immediate_process_failure_is_not_reported_as_timeout(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "project").mkdir()
            code, output, timed_out = runner.run_command(
                ["/bin/sh", "-c", "printf 'fixture startup failure\\n' >&2; exit 7"],
                root, {"PATH": os.defpath}, 2)
            self.assertFalse(timed_out)
            self.assertEqual(code, 7)
            self.assertEqual(output, "fixture startup failure\n")

    def test_zero_exit_without_rust_events_is_failure_and_preserves_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary, codex = root / "warp-tests", root / "codex"
            for path in (binary, codex):
                path.write_text("offline fixture")
                path.chmod(0o700)
            args = SimpleNamespace(test_binary=binary, codex=codex, timeout=300, output=root / "events.ndjson")
            command_results = [(0, "codex-cli 0.147.0\n", False), (0, self.output, False)]
            git_result = subprocess.CompletedProcess([], 0, "offline-fixture", "")
            with patch.object(sys, "platform", "linux"), patch.object(runner, "run_command", side_effect=command_results), \
                    patch.object(runner.subprocess, "run", return_value=git_result), patch("builtins.print"):
                self.assertEqual(runner.run(args), 1)
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertFalse(metadata["accepted"])
            self.assertFalse(metadata["private_root_removed"])
            private_root = Path(metadata["private_root"])
            self.assertTrue(private_root.is_dir())
            runner.shutil.rmtree(private_root)


    def test_windows_acceptance_requires_platform_gate_and_true_first_but_no_repeat_command(self):
        events = complete_events("win32")
        self.assertTrue(runner.verified_acceptance(0, self.output, events, "win32"))
        self.assertFalse(runner.verified_acceptance(0, self.output, events, "linux"))
        self.assertFalse(runner.verified_acceptance(0, self.output, complete_events(), "win32"))
        for index, key, value in ((0, "schema_version", 1), (0, "platform", "linux"),
                                   (3, "native_command_invoked", False), (5, "native_command_invoked", True),
                                   (11, "windows_product_gate_opened", False),
                                   (11, "windows_product_gate_opened", 1), (7, "case", [])):
            changed = complete_events("win32")
            changed[index][key] = value
            self.assertFalse(runner.verified_acceptance(0, self.output, changed, "win32"), key)

    def test_windows_input_gate_requires_exact_native_name_and_existing_official_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary, codex = root / "warp-tests.exe", root / "codex.exe"
            for path in (binary, codex):
                path.write_bytes(b"offline fixture")
                path.chmod(0o700)
            args = SimpleNamespace(test_binary=binary, codex=codex, timeout=300, output=root / "events.ndjson")
            with patch.object(sys, "platform", "win32"), patch.object(runner.platform, "machine", return_value="AMD64"):
                # 假字节不能只凭名字和版本通过；此路径不启动任何进程。
                with self.assertRaisesRegex(ValueError, "摘要"):
                    runner.validate_inputs(args, root / "repo")
                with patch.object(runner, "verify_codex") as verify:
                    runner.validate_inputs(args, root / "repo")
                    verify.assert_called_once_with(codex, "x86_64")
                shim = root / "codex.cmd"
                shim.write_bytes(b"offline fixture")
                shim.chmod(0o700)
                args.codex = shim
                with self.assertRaisesRegex(ValueError, "codex.exe"):
                    runner.validate_inputs(args, root / "repo")

    def test_windows_environment_retains_only_system_requirements_and_private_homes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            inherited = {"PATH": "offline-path", "SystemRoot": "C:\\Windows", "WINDIR": "C:\\Windows",
                         "COMSPEC": "C:\\Windows\\System32\\cmd.exe", "PATHEXT": ".COM;.EXE;.CMD",
                         "APPDATA": "do-not-inherit", "LOCALAPPDATA": "do-not-inherit",
                         "OPENAI_API_KEY": "do-not-inherit", "CODEX_AUTH_JSON": "do-not-inherit",
                         "BASH_ENV": "do-not-inherit", "ENV": "do-not-inherit", "HTTP_PROXY": "do-not-inherit"}
            with patch.dict(os.environ, inherited, clear=True), patch.object(sys, "platform", "win32"):
                environment = runner.isolated_environment(root, root / "native/codex.exe")
            for key in ("COMSPEC", "PATHEXT", "WINDIR"):
                self.assertEqual(environment[key], inherited[key])
            self.assertEqual(environment["SYSTEMROOT"], inherited["SystemRoot"])
            self.assertEqual(environment["APPDATA"], str(root / "home/AppData/Roaming"))
            self.assertEqual(environment["LOCALAPPDATA"], str(root / "home/AppData/Local"))
            self.assertEqual((root / ".infinishell-codex-source-probe").read_bytes(), runner.MARKER.encode())
            for key in ("OPENAI_API_KEY", "CODEX_AUTH_JSON", "BASH_ENV", "ENV", "HTTP_PROXY"):
                self.assertNotIn(key, environment)
            self.assertFalse((root / "codex/auth.json").exists())

    def test_windows_missing_system_environment_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.dict(os.environ, {"PATH": "offline"}, clear=True), patch.object(sys, "platform", "win32"):
                with self.assertRaisesRegex(ValueError, "系统环境"):
                    runner.isolated_environment(Path(temporary), Path(temporary) / "codex.exe")

    def test_windows_timeout_uses_owned_pid_tree_attempt_and_preserves_failure(self):
        process = Mock(pid=12345, returncode=1)
        process.communicate.side_effect = [subprocess.TimeoutExpired("offline", 1), ("partial diagnostic", None)]
        environment = {"SYSTEMROOT": "offline-system"}
        with patch.object(sys, "platform", "win32"), patch.object(runner.subprocess, "Popen", return_value=process) as launch, \
                patch.object(runner.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as taskkill:
            code, output, timed_out = runner.run_command(["offline-executable"], Path("offline-root"), environment, 1)
        self.assertTrue(timed_out)
        self.assertEqual(code, 1)
        self.assertEqual(output, "partial diagnostic")
        self.assertEqual(launch.call_args.kwargs["creationflags"], 0x00000200)
        self.assertNotIn("start_new_session", launch.call_args.kwargs)
        self.assertEqual(taskkill.call_args.args[0], [str(Path("offline-system") / "System32/taskkill.exe"),
                                                      "/PID", "12345", "/T", "/F"])
        self.assertEqual(taskkill.call_args.kwargs["timeout"], 10)


if __name__ == "__main__":
    unittest.main()
