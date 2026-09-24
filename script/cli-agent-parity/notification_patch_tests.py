#!/usr/bin/env python3
"""验证离线安装事务及配置保护；构造夹具不计作真实 CLI 生命周期验收。"""

import importlib.util
import json
from pathlib import Path
import os
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location("notification_patch", Path(__file__).with_name("apply_notification_patch.py"))
PATCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PATCH)


class NotificationPatchTests(unittest.TestCase):
    def fixture(self, directory):
        root = Path(directory)
        (root / "scripts").mkdir(parents=True)
        metadata = {"files": {}, "compatible_bases": [{"version": "test", "tree_sha256": {}}]}
        replacements = {}
        for suffix in ("a", "b"):
            name = "scripts/" + suffix
            original = ("original-" + suffix).encode()
            replacement = ("replacement-" + suffix).encode()
            (root / name).write_bytes(original)
            (root / name).chmod(0o755)
            metadata["files"][name] = {"upstream_sha256": PATCH.digest(original), "replacement_sha256": PATCH.digest(replacement)}
            metadata["compatible_bases"][0]["tree_sha256"][name] = PATCH.digest(original)
            replacements[name] = replacement
        return root, metadata, replacements

    def hook_fixture(self, directory, agent):
        root = Path(directory) / "应用 数据 ' $() ` 配置" / agent
        root.mkdir(parents=True)
        _, replacements = PATCH.bundle_data(PATCH.default_bundle(), agent)
        metadata = {"files": {}}
        originals = {}
        for name, replacement in replacements.items():
            original = ("original " + name).encode()
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(original)
            metadata["files"][name] = {"upstream_sha256": PATCH.digest(original), "replacement_sha256": PATCH.digest(replacement)}
            originals[name] = original
        return root, metadata, originals, replacements

    def test_hook_manifest_failure_restores_scripts_in_spaced_chinese_path(self):
        for agent in PATCH.CONTRACTS:
            with self.subTest(agent=agent), tempfile.TemporaryDirectory() as temporary:
                root, metadata, originals, replacements = self.hook_fixture(temporary, agent)
                real_write = PATCH.atomic_write

                def fail_hook(path, contents, mode, *, staging_dir=None):
                    if path.name == "hooks.json":
                        raise OSError("注入 hooks 清单替换失败")
                    return real_write(path, contents, mode, staging_dir=staging_dir)

                with mock.patch.object(PATCH, "atomic_write", fail_hook):
                    with self.assertRaises(OSError):
                        PATCH.apply_files(root, metadata, replacements)
                self.assertEqual({name: (root / name).read_bytes() for name in originals}, originals)
                PATCH.apply_files(root, metadata, replacements)
                PATCH.apply_files(root, metadata, replacements)
                self.assertEqual({name: (root / name).read_bytes() for name in replacements}, replacements)
                self.assertEqual(len([path for path in root.rglob("*") if path.is_file()]), len(replacements))

    def test_custom_hook_manifest_is_rejected_before_any_script_replacement(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, originals, replacements = self.hook_fixture(temporary, "claude")
            (root / "hooks/hooks.json").write_bytes(b'{"hooks":{"Custom":[]}}')
            with self.assertRaises(ValueError):
                PATCH.apply_files(root, metadata, replacements)
            self.assertEqual((root / "scripts/build-payload.sh").read_bytes(), originals["scripts/build-payload.sh"])
            self.assertEqual((root / "hooks/hooks.json").read_bytes(), b'{"hooks":{"Custom":[]}}')

    def test_notify_failure_restores_hooks_and_scripts_in_spaced_chinese_path(self):
        for agent in PATCH.CONTRACTS:
            with self.subTest(agent=agent), tempfile.TemporaryDirectory() as temporary:
                root, metadata, originals, replacements = self.hook_fixture(temporary, agent)
                real_write = PATCH.atomic_write

                def fail_notify(path, contents, mode, *, staging_dir=None):
                    if path.name == "warp-notify.sh":
                        raise OSError("注入 tmux 通知替换失败")
                    return real_write(path, contents, mode, staging_dir=staging_dir)

                with mock.patch.object(PATCH, "atomic_write", fail_notify):
                    with self.assertRaises(OSError):
                        PATCH.apply_files(root, metadata, replacements)
                self.assertEqual({name: (root / name).read_bytes() for name in originals}, originals)

    def test_custom_notify_is_rejected_before_other_replacements(self):
        for agent in PATCH.CONTRACTS:
            with self.subTest(agent=agent), tempfile.TemporaryDirectory() as temporary:
                root, metadata, originals, replacements = self.hook_fixture(temporary, agent)
                (root / "scripts/warp-notify.sh").write_bytes(b"custom notification")
                with self.assertRaises(ValueError):
                    PATCH.apply_files(root, metadata, replacements)
                for name, original in originals.items():
                    expected = b"custom notification" if name == "scripts/warp-notify.sh" else original
                    self.assertEqual((root / name).read_bytes(), expected)

    def test_idempotent_apply_and_whole_tree_validation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, replacements = self.fixture(temporary)
            original_mode = (root / "scripts/a").stat().st_mode & 0o777
            PATCH.validate_tree(root, "test", metadata)
            PATCH.apply_files(root, metadata, replacements)
            PATCH.validate_tree(root, "test", metadata)
            PATCH.apply_files(root, metadata, replacements)
            self.assertEqual((root / "scripts/a").read_bytes(), replacements["scripts/a"])
            self.assertEqual((root / "scripts/a").stat().st_mode & 0o777, original_mode)
            self.assertEqual(len(list((root / "scripts").iterdir())), 2)

    def test_process_exit_before_forward_or_rollback_rename_keeps_tree_retryable(self):
        child = r"""
import importlib.util, json, os, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location("notification_patch", sys.argv[1])
patch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(patch)
root, parent = Path(sys.argv[2]), Path(sys.argv[3])
metadata, phase = json.loads(sys.argv[4]), sys.argv[5]
replacements = {"scripts/a": b"replacement-a", "scripts/b": b"replacement-b"}
real_replace = patch.os.replace

def interrupted_replace(source, destination):
    target = Path(destination)
    if phase == "forward_first" or (phase == "forward_second" and target.name == "b"):
        os._exit(91)
    if phase == "rollback":
        if target.name == "b":
            raise OSError("注入第二次替换失败以触发回滚")
        if Path(source).read_bytes() == b"original-a":
            os._exit(91)
    return real_replace(source, destination)

patch.os.replace = interrupted_replace
patch.apply_files(root, metadata, replacements, staging_parent=parent)
raise AssertionError("未到达注入的进程中断点")
"""
        for phase, leftover in (("forward_first", b"replacement-a"), ("forward_second", b"replacement-b"), ("rollback", b"original-a")):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as temporary:
                parent = Path(temporary) / "plugins"
                cache = parent / "cache/claude-code-warp/warp"
                root, metadata, replacements = self.fixture(cache / "test")
                original_modes = {name: (root / name).stat().st_mode & 0o777 for name in replacements}
                result = subprocess.run([sys.executable, "-I", "-c", child, str(Path(PATCH.__file__).resolve()), str(root), str(parent), json.dumps(metadata), phase], capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 91, result.stderr.decode(errors="replace"))
                self.assertEqual(result.stdout, b"")
                self.assertEqual(result.stderr, b"")
                abandoned = list(parent.glob(".infinishell-notification-patch-*"))
                self.assertEqual(len(abandoned), 1)
                self.assertFalse(abandoned[0].is_relative_to(root))
                self.assertEqual(abandoned[0].stat().st_dev, root.stat().st_dev)
                if os.name == "posix":
                    self.assertEqual(abandoned[0].stat().st_mode & 0o777, 0o700)
                orphans = list(abandoned[0].iterdir())
                self.assertEqual(len(orphans), 1)
                self.assertEqual(orphans[0].read_bytes(), leftover)
                self.assertEqual(list(cache.iterdir()), [root])
                expected_a = b"original-a" if phase == "forward_first" else b"replacement-a"
                self.assertEqual((root / "scripts/a").read_bytes(), expected_a)
                self.assertEqual((root / "scripts/b").read_bytes(), b"original-b")
                PATCH.validate_tree(root, "test", metadata)
                PATCH.apply_files(root, metadata, replacements, staging_parent=parent)
                PATCH.validate_tree(root, "test", metadata)
                self.assertEqual({name: (root / name).read_bytes() for name in replacements}, replacements)
                self.assertEqual({name: (root / name).stat().st_mode & 0o777 for name in replacements}, original_modes)
                self.assertEqual(list(parent.glob(".infinishell-notification-patch-*")), abandoned)
                self.assertEqual(orphans[0].read_bytes(), leftover)

    def test_staging_inside_active_tree_is_rejected_without_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, replacements = self.fixture(temporary)
            with self.assertRaisesRegex(ValueError, "活动插件目录之外"):
                PATCH.apply_files(root, metadata, replacements, staging_parent=root)
            self.assertEqual((root / "scripts/a").read_bytes(), b"original-a")
            self.assertEqual(list(root.iterdir()), [root / "scripts"])

    @unittest.skipUnless(os.name == "posix", "符号链接行为限 Unix")
    def test_staging_alias_into_active_tree_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root, metadata, replacements = self.fixture(parent / "active")
            alias = parent / "alias"
            alias.symlink_to(root, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "活动插件目录之外"):
                PATCH.apply_files(root, metadata, replacements, staging_parent=alias)
            self.assertEqual((root / "scripts/a").read_bytes(), b"original-a")
            PATCH.validate_tree(root, "test", metadata)

    def test_other_filesystem_is_rejected_before_any_replace(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root, metadata, replacements = self.fixture(parent / "active")
            real_stat = Path.stat

            def other_device(path, *args, **kwargs):
                information = real_stat(path, *args, **kwargs)
                if path.name.startswith(".infinishell-notification-patch-"):
                    return SimpleNamespace(st_dev=information.st_dev + 1)
                return information

            with mock.patch.object(Path, "stat", other_device), mock.patch.object(PATCH, "atomic_write", side_effect=AssertionError("跨文件系统时不应写入")):
                with self.assertRaisesRegex(ValueError, "同一文件系统"):
                    PATCH.apply_files(root, metadata, replacements, staging_parent=parent)
            self.assertEqual((root / "scripts/a").read_bytes(), b"original-a")
            self.assertEqual((root / "scripts/b").read_bytes(), b"original-b")
            self.assertEqual(list(parent.iterdir()), [root])

    def test_old_unknown_temporary_inside_active_tree_is_not_accepted_or_deleted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, _ = self.fixture(temporary)
            unknown = root / "scripts/.infinishell-patch-user-file"
            unknown.write_bytes(b"unknown content")
            with self.assertRaisesRegex(ValueError, "自定义文件"):
                PATCH.validate_tree(root, "test", metadata)
            self.assertEqual(unknown.read_bytes(), b"unknown content")
            self.assertEqual((root / "scripts/a").read_bytes(), b"original-a")

    def test_custom_script_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, replacements = self.fixture(temporary)
            (root / "scripts/b").write_text("custom")
            with self.assertRaises(ValueError):
                PATCH.apply_files(root, metadata, replacements)
            self.assertEqual((root / "scripts/a").read_text(), "original-a")
            self.assertEqual((root / "scripts/b").read_text(), "custom")

    def test_unknown_extra_file_blocks_native_update_preflight(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, _ = self.fixture(temporary)
            (root / "custom-hook.sh").write_text("user content")
            with self.assertRaises(ValueError):
                PATCH.validate_tree(root, "test", metadata)

    def test_atomic_failure_rolls_back_only_changed_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, replacements = self.fixture(temporary)
            real_write = PATCH.atomic_write
            count = 0

            def fail_second(path, contents, mode, *, staging_dir=None):
                nonlocal count
                count += 1
                if count == 2:
                    raise OSError("注入写入失败")
                return real_write(path, contents, mode, staging_dir=staging_dir)

            with mock.patch.object(PATCH, "atomic_write", fail_second):
                with self.assertRaises(OSError):
                    PATCH.apply_files(root, metadata, replacements)
            self.assertEqual((root / "scripts/a").read_text(), "original-a")
            self.assertEqual((root / "scripts/b").read_text(), "original-b")

    def test_rollback_preserves_concurrent_edit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, replacements = self.fixture(temporary)
            real_write = PATCH.atomic_write

            def concurrent_edit(path, contents, mode, *, staging_dir=None):
                if path.name == "b":
                    (root / "scripts/a").write_text("concurrent")
                    raise OSError("注入并发编辑")
                return real_write(path, contents, mode, staging_dir=staging_dir)

            with mock.patch.object(PATCH, "atomic_write", concurrent_edit):
                with self.assertRaisesRegex(ValueError, "恢复失败"):
                    PATCH.apply_files(root, metadata, replacements)
            self.assertEqual((root / "scripts/a").read_text(), "concurrent")

    @unittest.skipUnless(os.name == "posix", "符号链接行为限 Unix")
    def test_link_cannot_redirect_patch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, metadata, replacements = self.fixture(temporary)
            external = root / "outside"
            external.write_text("original-a")
            (root / "scripts/a").unlink()
            (root / "scripts/a").symlink_to(external)
            with self.assertRaises(ValueError):
                PATCH.apply_files(root, metadata, replacements)
            self.assertEqual(external.read_text(), "original-a")

    def test_disabled_codex_config_is_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            settings = '[plugins."warp@codex-warp"]\nenabled = false\n\n[other]\nvalue = "keep"\n'
            (home / "config.toml").write_text(settings)
            with self.assertRaisesRegex(ValueError, "未启用"):
                PATCH.installation(home, "codex")
            self.assertEqual((home / "config.toml").read_text(), settings)

    def test_disabled_claude_config_is_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            (home / "plugins").mkdir()
            (home / "plugins/installed_plugins.json").write_text(json.dumps({"plugins": {"warp@claude-code-warp": [{"scope": "user", "installPath": "/unvisited", "version": "2.2.0"}]}}))
            settings = '{"enabledPlugins":{"warp@claude-code-warp":false},"other":"keep"}'
            (home / "settings.json").write_text(settings)
            with self.assertRaisesRegex(ValueError, "禁用"):
                PATCH.installation(home, "claude")
            self.assertEqual((home / "settings.json").read_text(), settings)

    def test_export_is_self_contained_and_does_not_include_other_agents(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "export"
            PATCH.export_bundle(PATCH.default_bundle(), destination)
            self.assertTrue((destination / "apply_notification_patch.py").is_file())
            self.assertFalse((destination / "grok").exists())
            manifest = json.loads((destination / "SHA256SUMS.json").read_text())
            for name, expected in manifest.items():
                self.assertEqual(PATCH.digest((destination / name).read_bytes()), expected)
            for agent in PATCH.CONTRACTS:
                PATCH.bundle_data(destination, agent)

    def test_current_cli_versions_are_macos_only_and_exact(self):
        for host in ("darwin", "linux", "win32"):
            for agent, output in PATCH.MACOS_CLI_VERSIONS.items():
                with self.subTest(host=host, agent=agent), \
                        mock.patch.object(PATCH.sys, "platform", host), \
                        mock.patch.object(PATCH.subprocess, "run", return_value=mock.Mock(stdout=output)):
                    if host == "darwin":
                        self.assertEqual(PATCH.verify_runtime(agent, Path("isolated-home"), agent, True), output)
                    else:
                        with self.assertRaisesRegex(ValueError, "已验证版本"):
                            PATCH.verify_runtime(agent, Path("isolated-home"), agent, True)
        for agent, output in (("codex", "codex-cli 0.156.2"),
                              ("codex", "codex-cli 0.156.1-beta"),
                              ("claude", "2.1.281 (Claude Code)"),
                              ("claude", "2.1.280-beta (Claude Code)"),
                              ("claude", "2.1.280 (Grok Build)")):
            with self.subTest(agent=agent, output=output), \
                    mock.patch.object(PATCH.sys, "platform", "darwin"), \
                    mock.patch.object(PATCH.subprocess, "run", return_value=mock.Mock(stdout=output)), \
                    self.assertRaisesRegex(ValueError, "已验证版本"):
                PATCH.verify_runtime(agent, Path("isolated-home"), agent, True)

    def test_previous_cli_contracts_remain_accepted_on_every_platform(self):
        for host in ("darwin", "linux", "win32"):
            for agent, contract in PATCH.CONTRACTS.items():
                with self.subTest(host=host, agent=agent), \
                        mock.patch.object(PATCH.sys, "platform", host), \
                        mock.patch.object(PATCH.subprocess, "run", return_value=mock.Mock(stdout=contract[0])):
                    self.assertEqual(PATCH.verify_runtime(agent, Path("isolated-home"), agent, True), contract[0])


if __name__ == "__main__":
    unittest.main()
