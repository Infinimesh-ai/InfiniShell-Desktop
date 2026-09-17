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
from unittest.mock import patch

import run_codex_source_live as runner


def complete_events():
    return [
        {"event": "rust_source_migration_started", "scope": "production_rust_installer",
         "credentials_provided": False, "model_commands_sent": 0},
        {"event": "production_runtime_probe", "succeeded": True},
        {"event": "rev3_fixture_verified", "source_files": 36, "cache_files": 10,
         "initial_installation": "controlled_exact_rev3_fixture", "trust_fixture_is_not_native_authorization": True},
        {"event": "install_observed", "phase": "rev3_to_rev4", "succeeded": True},
        {"event": "migration_verified", "rev4_source_and_cache_verified": True,
         "old_source_and_metadata_preserved": True, "old_cache_and_transaction_preserved": True,
         "user_configuration_and_trust_preserved": True, "disabled_orchestration_preserved": True,
         "transaction_phase": "verified"},
        {"event": "install_observed", "phase": "repeat_rev4_install", "succeeded": True},
        {"event": "repeat_install_verified", "previous_recovery_material_preserved": True},
        *[{"event": "rejection_observed", "case": case, "rejected": True,
           "bytes_and_modes_unchanged": True, "native_command_invoked": False, "rev4_source_created": False}
          for case in ("modified_rev3_source", "mixed_cache", "unknown_source", "disabled_target")],
        {"event": "rust_source_migration_passed", "passed": True, "model_commands_sent": 0,
         "hook_authorization_performed": False, "native_hook_execution_verified": False,
         "windows_product_gate_opened": False, "gui_verified": False, "rejected_cases": 4},
    ]


class SourceRunnerTests(unittest.TestCase):
    output = "test result: ok. 1 passed; 0 failed; 0 ignored; 7310 filtered out\n"

    def test_acceptance_requires_exact_test_and_complete_evidence(self):
        self.assertTrue(runner.verified_acceptance(0, self.output, complete_events()))
        for code, output in ((1, self.output), (0, "test result: ok. 0 passed; 0 failed; 1 ignored;")):
            self.assertFalse(runner.verified_acceptance(code, output, complete_events()))
        for index in range(len(complete_events())):
            events = complete_events()
            events.pop(index)
            self.assertFalse(runner.verified_acceptance(0, self.output, events), index)

    def test_rejections_require_all_distinct_cases_and_unchanged_bytes(self):
        for key, value in (("case", "modified_rev3_source"), ("rejected", False),
                           ("bytes_and_modes_unchanged", False), ("native_command_invoked", True),
                           ("rev4_source_created", True)):
            events = complete_events()
            events[8][key] = value
            self.assertFalse(runner.verified_acceptance(0, self.output, events), key)

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
            self.assertFalse(runner.verified_acceptance(0, self.output, events), key)

    def test_duplicate_old_or_reordered_records_are_rejected(self):
        events = complete_events()
        self.assertFalse(runner.verified_acceptance(0, self.output, events + copy.deepcopy(events)))
        events[3], events[5] = events[5], events[3]
        self.assertFalse(runner.verified_acceptance(0, self.output, events))

    def test_private_environment_drops_credentials_and_shell_injection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            inherited = {"PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8", "OPENAI_API_KEY": "do-not-inherit",
                         "ANTHROPIC_AUTH_TOKEN": "do-not-inherit", "BASH_ENV": "/user/startup",
                         "HOME": "/user/home", "CODEX_HOME": "/user/codex", "DYLD_INSERT_LIBRARIES": "injected"}
            with patch.dict(os.environ, inherited, clear=True):
                environment = runner.isolated_environment(root, root / "fixed/native/codex")
            self.assertEqual(environment["PATH"], str(root / "fixed/native") + os.pathsep + "/usr/bin:/bin")
            self.assertEqual(environment["HOME"], str(root / "home"))
            self.assertEqual(environment["CODEX_HOME"], str(root / "codex"))
            for key in ("OPENAI_API_KEY", "ANTHROPIC_AUTH_TOKEN", "BASH_ENV", "DYLD_INSERT_LIBRARIES"):
                self.assertNotIn(key, environment)
            self.assertEqual((root / ".infinishell-codex-source-probe").read_text(), runner.MARKER)
            self.assertEqual((root / "codex/config.toml").read_text(), 'cli_auth_credentials_store = "file"\n')
            self.assertFalse((root / "codex/auth.json").exists())

    def test_windows_remains_outside_production_gate(self):
        with patch.object(sys, "platform", "win32"):
            with self.assertRaisesRegex(ValueError, "Windows"):
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

    @unittest.skipUnless(os.name == "posix", "此运行器只支持生产 Unix 门控")
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

    @unittest.skipUnless(os.name == "posix", "此运行器只支持生产 Unix 门控")
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


if __name__ == "__main__":
    unittest.main()
