"""实际控制终端测试；复制缓存夹具不等于 Grok 原生安装验收。"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import stat
from types import SimpleNamespace
from unittest.mock import patch
import unittest

from run_installed_grok_hook import plain_file, verify_installed_hook


@unittest.skipUnless(os.name == "posix", "Windows CONOUT$ 需要独立原生验证")
class InstalledGrokHookTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="infinishell-hook-main-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        for path in ("home", "home/.grok", "tmp", "cache/hooks"):
            (self.root / path).mkdir(mode=0o700, parents=True, exist_ok=True)
        self.node = Path(shutil.which("node")).resolve(strict=True)
        self.node_sha = hashlib.sha256(self.node.read_bytes()).hexdigest()
        source = Path(__file__).resolve().parents[2] / "app/assets/bundled/cli-agent-plugins/grok"
        self.hook = self.root / "cache/hooks/notify.cjs"
        shutil.copyfile(source / "hooks/notify.cjs", self.hook)
        self.hook_sha = hashlib.sha256(self.hook.read_bytes()).hexdigest()
        self.version = json.loads((source / ".grok-plugin/plugin.json").read_bytes())["version"]
        self.environment = {"HOME": str(self.root / "home"), "GROK_HOME": str(self.root / "home/.grok"),
                            "TMPDIR": str(self.root / "tmp"), "PATH": "/usr/bin:/bin"}

    def verify(self):
        return verify_installed_hook(self.node, self.node_sha, self.hook, self.hook_sha,
                                     self.root, self.version, self.environment)

    def test_real_main_writes_version_to_controlling_terminal_and_keeps_stdout_empty(self):
        result = self.verify()
        self.assertTrue(result["main_entry_verified"])
        self.assertEqual(result["notification_channel"], "unix_controlling_tty")
        self.assertEqual(result["plugin_version"], "0.1.1")
        self.assertEqual(result["stdout_bytes"], 0)
        self.assertEqual(result["stderr_bytes"], 0)

    def test_modified_cache_is_rejected_before_process_launch(self):
        self.hook.write_text("throw new Error('cache mutation');")
        with self.assertRaisesRegex(ValueError, "hook_file_identity_invalid"):
            self.verify()
        self.assertFalse((self.root / "installed-hook-main-data").exists())

    def test_stdout_notification_is_not_a_terminal_receipt(self):
        self.hook.write_text("process.stdout.write('notification');")
        self.hook_sha = hashlib.sha256(self.hook.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, "hook_main_channel_invalid"):
            self.verify()

    def test_extra_environment_is_rejected_before_process_launch(self):
        for key in ("PASSWORD", "NODE_OPTIONS"):
            with self.subTest(key=key):
                environment = {**self.environment, key: "isolated-test-placeholder"}
                with self.assertRaisesRegex(ValueError, "hook_environment_not_isolated"):
                    verify_installed_hook(self.node, self.node_sha, self.hook, self.hook_sha,
                                          self.root, self.version, environment)
                self.assertFalse((self.root / "installed-hook-main-data").exists())

    def test_system_executable_owner_never_relaxes_private_hook_ownership(self):
        # 只模拟文件元数据，不需要 chown 或 root 权限；不启动子进程。
        for owner, mode, executable, accepted in [
                (1001, 0o755, True, True), (0, 0o755, True, True),
                (0, 0o777, True, False), (0, 0o644, True, False),
                (1002, 0o755, True, False), (0, 0o755, False, False)]:
            with self.subTest(owner=owner, mode=mode, executable=executable):
                info = SimpleNamespace(st_mode=stat.S_IFREG | mode, st_nlink=1, st_uid=owner)
                with patch("run_installed_grok_hook.os.getuid", return_value=1001), \
                        patch("run_installed_grok_hook.Path.lstat", return_value=info):
                    if accepted:
                        self.assertEqual(plain_file(self.hook, self.hook_sha, executable=executable), self.hook)
                    else:
                        with self.assertRaisesRegex(ValueError, "hook_file_identity_invalid"):
                            plain_file(self.hook, self.hook_sha, executable=executable)

    def test_credential_environment_is_rejected_before_process_launch(self):
        self.environment["API_KEY"] = "isolated-test-placeholder"
        with self.assertRaisesRegex(ValueError, "hook_environment_not_isolated"):
            self.verify()
        self.assertFalse((self.root / "installed-hook-main-data").exists())


if __name__ == "__main__":
    unittest.main()
