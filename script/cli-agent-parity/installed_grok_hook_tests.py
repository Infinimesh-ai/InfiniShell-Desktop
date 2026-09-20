"""实际控制终端测试；复制缓存夹具不等于 Grok 原生安装验收。"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import stat
import subprocess
import time
import shlex
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
        worker = os.environ.get("INFINISHELL_TEST_NOTIFY_WORKER")
        if not worker:
            self.fail("INFINISHELL_TEST_NOTIFY_WORKER 必须指向同源码构建的原生 worker")
        self.worker = Path(worker).resolve(strict=True)
        self.worker_sha = hashlib.sha256(self.worker.read_bytes()).hexdigest()
        source = Path(__file__).resolve().parents[2] / "app/assets/bundled/cli-agent-plugins/grok"
        self.hook = self.root / "cache/hooks/notify.cjs"
        shutil.copyfile(source / "hooks/notify.cjs", self.hook)
        self.hook_sha = hashlib.sha256(self.hook.read_bytes()).hexdigest()
        self.version = json.loads((source / ".grok-plugin/plugin.json").read_bytes())["version"]
        self.environment = {"HOME": str(self.root / "home"), "GROK_HOME": str(self.root / "home/.grok"),
                            "TMPDIR": str(self.root / "tmp"), "PATH": "/usr/bin:/bin"}

    def verify(self):
        return verify_installed_hook(self.node, self.node_sha, self.hook, self.hook_sha,
                                     self.worker, self.worker_sha, self.root, self.version,
                                     self.environment)

    def test_real_main_writes_version_to_controlling_terminal_and_keeps_stdout_empty(self):
        result = self.verify()
        self.assertTrue(result["main_entry_verified"])
        self.assertEqual(result["notification_channel"], "native_worker_to_unix_controlling_tty")
        self.assertEqual(result["plugin_version"], "0.1.3")
        self.assertEqual(result["worker_sha256"], self.worker_sha)
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
                                          self.worker, self.worker_sha, self.root, self.version,
                                          environment)
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


@unittest.skipUnless(os.name == "posix", "Windows CONOUT$ 需要独立原生验证")
class DetachedGrokHookTests(unittest.TestCase):
    def setUp(self):
        import pty
        import tty
        self.temporary = tempfile.TemporaryDirectory(prefix="infinishell-detached-hook-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.repo = Path(__file__).resolve().parents[2]
        self.hook = self.repo / "app/assets/bundled/cli-agent-plugins/grok/hooks/notify.cjs"
        self.node = Path(shutil.which("node")).resolve(strict=True)
        worker = os.environ.get("INFINISHELL_TEST_NOTIFY_WORKER")
        if not worker:
            self.fail("INFINISHELL_TEST_NOTIFY_WORKER 必须指向同源码构建的原生 worker")
        self.worker = Path(worker).resolve(strict=True)
        self.terminals = []
        for unused in range(2):
            master, slave = pty.openpty()
            tty.setraw(slave)
            os.set_blocking(master, False)
            self.terminals.append((master, slave, os.ttyname(slave)))
            self.addCleanup(os.close, master)
            self.addCleanup(os.close, slave)
        self.env = {"HOME": str(self.root), "PATH": "/usr/bin:/bin", "TMPDIR": str(self.root),
                    "WARP_CLI_AGENT_PROTOCOL_VERSION": "1", "GROK_HOOK_EVENT": "session_start",
                    "GROK_SESSION_ID": "detached-terminal-test",
                    "WARP_CLI_AGENT_NOTIFY_EXECUTABLE": str(self.worker)}
        self.payload = json.dumps({"hookEventName": "session_start", "sessionId": "detached-terminal-test"}).encode()

    def run_hook(self, extra, hook=None):
        # 和 Grok 的 hook 一样真正 setsid 且三条标准流都是管道；先证明 /dev/tty 报 ENXIO。
        check = ("const fs=require('node:fs');try {const fd=fs.openSync('/dev/tty','w');"
                 "fs.closeSync(fd);process.exit(90)} catch(e) {if(e.code!=='ENXIO')process.exit(91)};"
                 "require(process.argv[1]).main();")
        result = subprocess.run([str(self.node), "-e", check, str(hook or self.hook)],
                                input=self.payload, capture_output=True, start_new_session=True,
                                env={**self.env, **extra}, timeout=5, check=True)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"")

    def output(self, index):
        result = bytearray()
        while True:
            try:
                chunk = os.read(self.terminals[index][0], 65536)
            except BlockingIOError:
                return bytes(result)
            if not chunk:
                return bytes(result)
            result.extend(chunk)
            self.assertLessEqual(len(result), 65536)

    def assert_notification(self, output, tmux=False):
        if tmux:
            self.assertTrue(output.startswith(b"\x1bPtmux;"))
            self.assertTrue(output.endswith(b"\x1b\\"))
            output = output[7:-2].replace(b"\x1b\x1b", b"\x1b")
        prefix = b"\x1b]777;notify;warp://cli-agent;"
        self.assertTrue(output.startswith(prefix))
        self.assertTrue(output.endswith(b"\x07"))
        event = json.loads(output[len(prefix):-1])
        self.assertEqual(event["event"], "session_start")
        self.assertEqual(event["session_id"], "detached-terminal-test")
        self.assertEqual(event["plugin_version"], "0.1.3")
        self.assertEqual(event["agent"], "grok")

    def check_bootstrap_refresh(self, shell, filename, marker, tail):
        executable = shutil.which(shell)
        if not executable:
            self.skipTest(f"需要真实 {shell}；缺失不算该 shell 验证通过")
        body = (self.repo / "app/assets/bundled/bootstrap" / filename).read_text()
        refresh = body[body.index(marker):].split("\n\n", 1)[0]
        options = ["--noprofile", "--norc"] if shell == "bash" else ["-f"] if shell == "zsh" else []
        result = subprocess.run([executable, *options, "-c", refresh + "\n" + tail],
                                stdin=self.terminals[0][1], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env={**self.env, "WARP_CLI_AGENT_TTY": self.terminals[1][2]},
                                start_new_session=True, timeout=5, check=True)
        self.assertEqual(result.stdout.decode(), self.terminals[0][2])
        self.assertEqual(result.stderr, b"")
        self.assertEqual(self.output(1), b"")

    def test_bash_bootstrap_replaces_inherited_terminal(self):
        self.check_bootstrap_refresh("bash", "bash_body.sh", "# 新 shell 和 SSH 必须刷新当前 PTY",
                                     'printf "%s" "$WARP_CLI_AGENT_TTY"')

    def test_zsh_bootstrap_replaces_inherited_terminal(self):
        self.check_bootstrap_refresh("zsh", "zsh_body.sh", "# 新 shell 和 SSH 必须刷新当前 PTY",
                                     'printf "%s" "$WARP_CLI_AGENT_TTY"')

    def test_fish_bootstrap_replaces_inherited_terminal(self):
        self.check_bootstrap_refresh("fish", "fish.sh", "# 新 shell 和 SSH 必须刷新当前 PTY",
                                     'printf "%s" "$WARP_CLI_AGENT_TTY"')

    def test_old_011_loses_notification_when_hook_has_no_controlling_terminal(self):
        self.run_hook({"WARP_CLI_AGENT_TTY": self.terminals[0][2]},
                      self.repo / "specs/cli-agent-parity/fixtures/grok-plugin-0.1.1-notify.cjs")
        self.assertEqual(self.output(0), b"")
        self.assertFalse((self.root / "data").exists())

    def test_refreshed_shell_terminal_beats_inherited_ssh_terminal(self):
        self.run_hook({"WARP_CLI_AGENT_TTY": self.terminals[0][2], "SSH_TTY": self.terminals[1][2]})
        self.assert_notification(self.output(0))
        self.assertEqual(self.output(1), b"")
        self.assertFalse((self.root / "data").exists())

    def test_ssh_terminal_without_bootstrap_still_uses_real_pty(self):
        self.run_hook({"SSH_TTY": self.terminals[0][2]})
        self.assert_notification(self.output(0))
        self.assertEqual(self.output(1), b"")

    def test_invalid_tmux_identity_never_writes_outer_or_ssh_terminal(self):
        self.run_hook({"TMUX": "/no-such-server,1,0", "TMUX_PANE": "invalid",
                       "WARP_CLI_AGENT_TTY": self.terminals[0][2], "SSH_TTY": self.terminals[1][2]})
        self.assertEqual(self.output(0), b"")
        self.assertEqual(self.output(1), b"")
        self.assertFalse((self.root / "data").exists())

    def test_failed_tmux_query_never_falls_back_to_outer_terminal(self):
        tmux = os.environ.get("INFINISHELL_TEST_TMUX") or shutil.which("tmux")
        search_path = str(Path(tmux).parent) + ":/usr/bin:/bin" if tmux else "/usr/bin:/bin"
        self.run_hook({"TMUX": "/no-such-server,1,0", "TMUX_PANE": "%1", "PATH": search_path,
                       "WARP_CLI_AGENT_TTY": self.terminals[0][2], "SSH_TTY": self.terminals[1][2]})
        self.assertEqual(self.output(0), b"")
        self.assertEqual(self.output(1), b"")
        self.assertFalse((self.root / "data").exists())

    def test_regular_file_and_symlink_are_not_notification_targets(self):
        target = self.root / "keep.txt"
        target.write_bytes(b"must stay unchanged")
        self.run_hook({"WARP_CLI_AGENT_TTY": str(target)})
        self.assertEqual(target.read_bytes(), b"must stay unchanged")
        alias = self.root / "alias"
        alias.symlink_to(self.terminals[0][2])
        self.run_hook({"WARP_CLI_AGENT_TTY": str(alias)})
        self.assertEqual(self.output(0), b"")
        self.assertFalse((self.root / "data").exists())

    def test_missing_or_non_terminal_device_does_not_create_state_or_emit_stdout(self):
        self.run_hook({})
        self.assertFalse((self.root / "data").exists())
        for target in ("/dev/infinishell-no-such-tty", "/dev/null"):
            with self.subTest(target=target):
                self.run_hook({"WARP_CLI_AGENT_TTY": target})
                self.assertFalse((self.root / "data").exists())
        self.assertEqual(self.output(0), b"")
        self.assertEqual(self.output(1), b"")

    def test_real_tmux_pane_wins_over_both_stale_outer_paths(self):
        tmux = os.environ.get("INFINISHELL_TEST_TMUX") or shutil.which("tmux")
        if not tmux:
            self.skipTest("需要真实 tmux；缺失不算 tmux 验证通过")
        tmux = str(Path(tmux).resolve(strict=True))
        socket = self.root / "tmux.sock"
        environment = {**self.env, "PATH": str(Path(tmux).parent) + ":/usr/bin:/bin"}
        def command(*args):
            return subprocess.run([tmux, "-S", str(socket), *args], env=environment,
                                  check=True, capture_output=True, text=True, timeout=5).stdout.strip()
        command("new-session", "-d", "-s", "hook", "-x", "80", "-y", "24", "/bin/cat")
        try:
            first = command("display-message", "-p", "-t", "hook", "#{pane_id}")
            second = command("split-window", "-h", "-P", "-F", "#{pane_id}", "-t", first, "/bin/cat")
            capture = self.root / "pane-output.bin"
            capture.touch()
            command("pipe-pane", "-t", second, "cat > " + shlex.quote(str(capture)))
            server = command("display-message", "-p", "-t", second, "#{pid}")
            self.run_hook({"PATH": environment["PATH"], "TMUX": f"{socket},{server},0", "TMUX_PANE": second,
                           "WARP_CLI_AGENT_TTY": self.terminals[0][2], "SSH_TTY": self.terminals[1][2]})
            deadline = time.monotonic() + 2
            while not capture.read_bytes().endswith(b"\x1b\\") and time.monotonic() < deadline:
                time.sleep(.01)
            self.assert_notification(capture.read_bytes(), tmux=True)
            self.assertEqual(self.output(0), b"")
            self.assertEqual(self.output(1), b"")
            self.assertEqual(command("capture-pane", "-p", "-t", first), "")
        finally:
            command("kill-server")


if __name__ == "__main__":
    unittest.main()
