#!/usr/bin/env python3
"""固定升级入口的离线边界；不下载、不执行 CLI、不访问账户配置。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

import run_fixed_cli_autoupdate as runner


class FixedUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.worker = self.root / "worker"
        self.worker.write_bytes(b"synthetic-test-binary")
        self.supervisor = self.root / "supervisor"
        self.supervisor.write_bytes(b"synthetic-supervisor")
        self.old = self.root / "old"
        self.target = self.root / "target"
        self.old.write_bytes(b"old-official-placeholder")
        self.target.write_bytes(b"new-official-placeholder")
        self.args = SimpleNamespace(test_binary=self.worker, supervisor=self.supervisor,
                                    fixture_parent=self.root, output=self.root / "summary.safe.json")

    def test_codex_copy_preserves_the_entire_package_and_refuses_overwrite(self):
        source = self.root / "input/bin/codex"
        source.parent.mkdir(parents=True)
        source.write_bytes(b"native")
        resource = self.root / "input/resources/payload"
        resource.parent.mkdir()
        resource.write_bytes(b"resource")
        destination = self.root / "clone/runtime/bin/codex"
        runner.copy_input("codex", source, destination)
        self.assertEqual(destination.read_bytes(), b"native")
        self.assertEqual((destination.parent.parent / "resources/payload").read_bytes(), b"resource")
        with self.assertRaises(FileExistsError):
            runner.copy_input("codex", source, destination)
        self.assertEqual(source.read_bytes(), b"native")

    @unittest.skipUnless(os.name == "posix", "Unix symlink 夹具")
    def test_unix_fixture_canonicalizes_only_inside_private_installation(self):
        with patch.object(runner.sys, "platform", "linux"):
            root, old, target, entry = runner.fixture("claude", self.old, self.target, self.root)
        self.assertEqual(entry.resolve(), old)
        self.assertTrue(root.name.startswith("infinishell-cli-autoupdate-"))
        self.assertEqual(target.read_bytes(), self.target.read_bytes())
        self.assertTrue(runner.transaction.auth_absent(root))
        self.assertEqual(old.parent.relative_to(root).as_posix(), "home/.local/share/claude/versions")

    def test_windows_environment_discards_credentials_and_overrides_all_cli_roots(self):
        with patch.dict(os.environ, {"SystemRoot": str(self.root / "Windows"), "SECRET_TOKEN": "synthetic",
                                    "ANTHROPIC_API_KEY": "synthetic", "CODEX_HOME": "/not-allowed",
                                    "PATH": "/not-allowed"}, clear=True):
            environment = runner.windows_environment(self.root / "fixture")
        self.assertNotIn("ANTHROPIC_API_KEY", environment)
        self.assertNotIn("SECRET_TOKEN", environment)
        self.assertNotIn("/not-allowed", environment.values())
        for name in ("HOME", "USERPROFILE", "CODEX_HOME", "GROK_HOME", "CLAUDE_CONFIG_DIR", "APPDATA", "LOCALAPPDATA"):
            self.assertTrue(Path(environment[name]).is_relative_to(self.root / "fixture"))
        self.assertEqual(environment["CODEX_RELEASE"], "0.156.1")

    def windows_case(self, *, native_code=0, target_installed=True, config_changed=False, version_correct=True):
        observed = {}

        def popen(command, **keywords):
            root = Path(keywords["cwd"]).parent
            env = keywords["env"]
            observed["arguments"] = json.loads(env["INFINISHELL_WINDOWS_REAL_CLI_ARGS"])
            observed["environment"] = json.loads(env["INFINISHELL_WINDOWS_REAL_CLI_ENV"])
            self.assertEqual(command[1], runner.WINDOWS_TEST)
            self.assertIn("--ignored", command)
            if target_installed:
                (root / "home/.local/bin/claude.exe").write_bytes(self.target.read_bytes())
            if config_changed:
                (root / "home/.claude/settings.json").write_text("changed")
            keywords["stdout"].write(("严格 Job 已清空\n1 passed; 0 failed\n"
                                      f"真实更新参数的原生退出状态：exit code: {native_code}\n").encode())
            return SimpleNamespace(wait=lambda **_: 0, returncode=0, pid=123)

        completed = subprocess.CompletedProcess([], 0, "2.1.280 (Claude Code)\n" if version_correct else "2.1.278 (Claude Code)\n", "")
        with patch.object(runner.sys, "platform", "win32"), patch.dict(os.environ, {"SystemRoot": str(self.root / "Windows")}):
            with patch.object(runner.subprocess, "Popen", side_effect=popen), patch.object(runner.subprocess, "run", return_value=completed):
                result = runner.windows_debug_case(self.args, "claude", self.old, self.target)
        evidence = json.loads(Path(result["receipt"]["path"]).read_bytes())
        return result, evidence, observed

    def test_windows_success_requires_real_target_bytes_version_and_zero_exit(self):
        result, evidence, observed = self.windows_case()
        self.assertTrue(result["passed"])
        self.assertTrue(evidence["candidate"])
        self.assertEqual(observed["arguments"], ["--settings", '{"autoUpdatesChannel":"latest"}', "install", "2.1.280"])
        self.assertNotIn("INFINISHELL_WINDOWS_REAL_CLI_ENV", observed["environment"])
        self.assertFalse(evidence["credentials_provided"])
        self.assertNotIn("private_log", evidence)
        self.assertTrue(evidence["private_log_sha256"])

    def test_windows_nonzero_exit_cannot_pass_after_target_was_written(self):
        result, evidence, _ = self.windows_case(native_code=1)
        self.assertFalse(result["passed"])
        self.assertTrue(evidence["entry_matches_target"])
        self.assertFalse(evidence["updater_exit_success"])

    def test_windows_native_exit_without_update_is_failure(self):
        result, evidence, _ = self.windows_case(target_installed=False)
        self.assertFalse(result["passed"])
        self.assertTrue(evidence["native_exit_and_strict_job_confirmed"])
        self.assertFalse(evidence["entry_matches_target"])

    def test_windows_changed_configuration_or_wrong_version_is_failure(self):
        for change in ({"config_changed": True}, {"version_correct": False}):
            with self.subTest(change=change):
                result, _, _ = self.windows_case(**change)
                self.assertFalse(result["passed"])

    def test_preparer_uses_only_requested_fixed_version(self):
        cache = self.root / "cache"
        cache.mkdir()
        executable = cache / "claude"
        executable.write_bytes(b"synthetic")
        completed = subprocess.CompletedProcess([], 0, str(executable), "")
        with patch.object(runner.subprocess, "run", return_value=completed) as run:
            self.assertEqual(runner.prepared_input("claude", "2.1.280", cache), executable)
        self.assertIn("2.1.280", run.call_args.args[0])
        self.assertNotIn("latest", run.call_args.args[0])
        with patch.object(runner.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, str(self.old), "")):
            with self.assertRaisesRegex(ValueError, "fixed_input_outside_cache"):
                runner.prepared_input("claude", "2.1.280", cache)

    @unittest.skipUnless(os.name == "posix", "Unix 插件准备夹具")
    def test_plugin_preparation_restores_entry_and_manifest_when_cleanup_rejects(self):
        with patch.object(runner.sys, "platform", "linux"):
            root, old, target, entry = runner.fixture("claude", self.old, self.target, self.root)
        value = {"agent": "claude", "entry": str(entry), "case_id": "claude-updated",
                 "worker": runner.binding(self.worker), "supervisor": runner.binding(self.supervisor),
                 "old_version": "2.1.278", "old_binary": runner.binding(old)}
        original = runner.transaction.encoded(value)
        manifest = root / "manifest.private.json"
        manifest.write_bytes(original)
        link = entry.readlink()

        def popen(command, **keywords):
            self.assertEqual(entry.read_bytes(), self.target.read_bytes())
            self.assertEqual(json.loads(manifest.read_bytes())["old_version"], "2.1.273")
            self.assertEqual(command[2], runner.PREPARE_TEST)
            self.assertEqual(keywords["env"]["HOME"], str(root / "home"))
            runner.write(root / "plugin-preparation.safe.json", {"passed": True})
            return SimpleNamespace(wait=lambda **_: 0)

        with patch.object(runner.subprocess, "Popen", side_effect=popen), patch.object(runner.transaction, "stop_group", return_value=False):
            with self.assertRaisesRegex(ValueError, "plugin_process_group_not_stopped"):
                runner.prepare_plugins(root, value, self.target)
        self.assertEqual(entry.readlink(), link)
        self.assertEqual(manifest.read_bytes(), original)
        self.assertEqual(old.read_bytes(), self.old.read_bytes())

    def product_case(self, change=None):
        def spawn(command, **keywords):
            self.assertEqual(command[1], runner.WINDOWS_PRODUCT_TEST)
            root = Path(keywords["cwd"]).parent
            manifest = Path(keywords["env"]["INFINISHELL_CLI_AUTOUPDATE_MANIFEST"])
            value = json.loads(manifest.read_bytes())
            Path(value["entry"]).write_bytes(self.target.read_bytes())
            product = {"scope": value["scope"], "agent": "claude", "old_version": "2.1.278", "target_version": "2.1.280",
                "manifest_sha256": runner.transaction.digest(manifest), "worker_sha256": value["worker"]["sha256"],
                "supervisor_sha256": value["supervisor"]["sha256"], "old_sha256": value["old_binary"]["sha256"],
                "target_sha256": value["target_binary"]["sha256"], "product_execute_calls": 1, "product_inspect_calls": 2,
                "config_files_checked": 13, "fixed_release_input": True, "capability_or_source_gate_bypassed": False,
                "model_inputs_sent": 0, "credentials_provided": False, "passed": True,
                **{name: True for name in ("native_exit_confirmed", "strict_job_cleanup_confirmed", "entry_matches_target", "target_version_matches",
                    "config_bytes_unchanged", "journal_absent", "old_binary_unchanged", "target_reference_unchanged", "credentials_absent")}}
            product.update(change or {})
            runner.write(root / "receipt.windows.safe.json", product)
            return SimpleNamespace(wait=lambda **_: 0, returncode=0, pid=123)
        source_root = self.root / "source"
        for relative in runner.WINDOWS_SOURCES:
            path = source_root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"synthetic-source")
        with patch.object(runner, "REPO", source_root), patch.object(runner.sys, "platform", "win32"):
            with patch.dict(os.environ, {"SystemRoot": str(self.root / "Windows")}):
                with patch.object(runner.subprocess, "Popen", side_effect=spawn):
                    return runner.windows_case(self.args, "claude", self.old, self.target)

    def test_windows_product_transaction_requires_bound_receipt_and_native_exit(self):
        result = self.product_case()
        self.assertTrue(result["passed"])
        self.assertTrue(result["product_receipt"])

    def test_windows_product_transaction_rejects_debug_only_or_bypassed_success(self):
        for change in ({"product_execute_calls": 0}, {"product_inspect_calls": 1}, {"native_exit_confirmed": False},
                       {"capability_or_source_gate_bypassed": True}, {"journal_absent": False}, {"manifest_sha256": "wrong"}):
            with self.subTest(change=change):
                self.assertFalse(self.product_case(change)["passed"])

    def registry(self, original):
        state = {"value": original, "writes": 0}
        key = MagicMock()
        key.__enter__.return_value = key

        def query(_key, name):
            self.assertEqual(name, "Path")
            if state["value"] is None:
                raise FileNotFoundError()
            return state["value"]

        def write(_key, name, _reserved, kind, value):
            self.assertEqual(name, "Path")
            state["value"] = (value, kind)
            state["writes"] += 1

        def delete(_key, name):
            self.assertEqual(name, "Path")
            state["value"] = None

        registry = SimpleNamespace(HKEY_CURRENT_USER=1, KEY_QUERY_VALUE=1, KEY_SET_VALUE=2,
                                   REG_SZ=1, REG_EXPAND_SZ=2, OpenKey=lambda *_: key,
                                   QueryValueEx=query, SetValueEx=write, DeleteValue=delete)
        return registry, state

    def test_windows_user_path_restores_exact_value_and_registry_type_after_failure(self):
        original = (r"%SystemRoot%\System32;;C:\existing", 2)
        registry, state = self.registry(original)
        with patch.dict(runner.sys.modules, {"winreg": registry}):
            with self.assertRaisesRegex(RuntimeError, "synthetic_child_failure"):
                with runner.windows_codex_user_path(self.root / "bin/codex.exe"):
                    self.assertTrue(state["value"][0].startswith(str(self.root / "bin") + ";"))
                    raise RuntimeError("synthetic_child_failure")
        self.assertEqual(state["value"], original)

    def test_windows_user_path_restores_absence(self):
        registry, state = self.registry(None)
        with patch.dict(runner.sys.modules, {"winreg": registry}):
            with runner.windows_codex_user_path(self.root / "bin/codex.exe"):
                self.assertEqual(state["value"], (str(self.root / "bin"), registry.REG_EXPAND_SZ))
        self.assertIsNone(state["value"])

    def test_windows_user_path_preserves_a_concurrent_change_and_fails(self):
        registry, state = self.registry(("original", 1))
        with patch.dict(runner.sys.modules, {"winreg": registry}):
            with self.assertRaisesRegex(ValueError, "windows_user_path_changed_concurrently"):
                with runner.windows_codex_user_path(self.root / "bin/codex.exe"):
                    state["value"] = ("concurrent", 1)
        self.assertEqual(state["value"], ("concurrent", 1))
        self.assertEqual(state["writes"], 1)

    def test_windows_user_path_rejects_non_string_before_writing(self):
        registry, state = self.registry((b"not-a-string", 3))
        with patch.dict(runner.sys.modules, {"winreg": registry}):
            with self.assertRaisesRegex(ValueError, "windows_user_path_type_unsupported"):
                with runner.windows_codex_user_path(self.root / "bin/codex.exe"):
                    self.fail("unsupported registry value entered native updater")
        self.assertEqual(state["writes"], 0)

    @unittest.skipUnless(os.name == "posix", "离线替代联接仅用来检查安装布局")
    def test_windows_codex_fixture_uses_two_junctions_and_default_visible_entry(self):
        source = self.root / "old-package/bin/codex.exe"
        source.parent.mkdir(parents=True)
        source.write_bytes(b"old")
        target = self.root / "target-package/bin/codex.exe"
        target.parent.mkdir(parents=True)
        target.write_bytes(b"target")
        with patch.object(runner.sys, "platform", "win32"):
            with patch.object(runner, "windows_junction", side_effect=lambda link, target: (link.parent.mkdir(parents=True, exist_ok=True), link.symlink_to(target, target_is_directory=True))):
                root, old, _, entry = runner.fixture("codex", source, target, self.root)
        self.assertEqual(entry.relative_to(root).as_posix(), "home/AppData/Local/Programs/OpenAI/Codex/bin/codex.exe")
        self.assertEqual(entry.resolve(), old)
        self.assertTrue((root / "home/.codex/packages/standalone/current").is_symlink())
        self.assertTrue(entry.parent.is_symlink())
        self.assertFalse((root / "home/.local/bin/codex.exe").exists())

    def test_windows_junction_rejects_command_expansion_before_spawn(self):
        with patch.object(runner.sys, "platform", "win32"):
            with patch.object(runner.subprocess, "run") as spawn:
                with self.assertRaisesRegex(ValueError, "junction_path_not_safe"):
                    runner.windows_junction(self.root / "%bad%", self.root)
                spawn.assert_not_called()

    def test_linux_candidate_pins_match_preparer_catalogs(self):
        for agent, (version, checksum) in runner.transaction.LINUX_FIXED_CANDIDATES.items():
            self.assertEqual(version, runner.TARGET[agent])
            if agent == "codex":
                package = runner.codex.packages_for_version(version)["linux-x64"]
                actual = package["files"][package["entrypoint"]][1]
            elif agent == "claude":
                actual = runner.claude.release_contract(version)["platforms"]["linux-x64"][2]
            else:
                actual = runner.grok.releases_for(version)["linux-x64"][3]
            self.assertEqual(checksum, actual)


if __name__ == "__main__":
    unittest.main()
