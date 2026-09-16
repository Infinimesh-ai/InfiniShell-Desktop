#!/usr/bin/env python3
"""用真实 Unix 控制终端检查通知字节；不冒充 Windows 或原生 CLI/UI 验收。"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest

from plugin_compatibility_tests import ASSETS, EMITTER


class TmuxNotificationTests(unittest.TestCase):
    def notify(self, agent, tmux, version=None, with_tty=True):
        if os.name != "posix":
            self.fail("此测试需要真实 Unix 控制终端；Windows 原生传输须另行验证")
        import fcntl
        import pty
        import termios
        import tty

        with tempfile.TemporaryDirectory(prefix="infinishell-notify-") as temporary:
            root = Path(temporary) / "插件 空 格 ' $(touch INJECTED)"
            shutil.copytree(ASSETS / agent / "scripts", root / "scripts")
            if agent == "claude":
                shutil.copyfile(EMITTER, root / "scripts/emit-terminal-sequence.sh")
            else:
                # 此替身只打开协议广告门槛；通知脚本使用完整随附件。
                (root / "scripts/should-use-structured.sh").write_text("should_use_structured() { return 0; }\n")
            environment = {key: value for key, value in os.environ.items() if key not in ("TMUX", "CLAUDE_CODE_VERSION", "GROK_HOOK_EVENT", "GROK_SESSION_ID")}
            environment.update(WARP_CLI_AGENT_PROTOCOL_VERSION="1", WARP_CLIENT_VERSION="infinishell-test-dev")
            if tmux:
                environment["TMUX"] = "isolated-byte-test"
            if version:
                environment["CLAUDE_CODE_VERSION"] = version
            body = '{"response":"中文 $(touch INJECTED) %s"}' + "\x1b]0;escape\x07"
            raw = ("\x1b]777;notify;warp://cli-agent;" + body + "\x07").encode()
            master, slave = pty.openpty()
            tty.setraw(slave)
            os.set_blocking(master, False)
            data = bytearray()
            finished = threading.Event()

            def drain_terminal():
                # macOS 在会话退出时可能等待控制终端排空，不能等 wait 完成才读取。
                while not finished.is_set():
                    try:
                        chunk = os.read(master, 65536)
                        data.extend(chunk)
                    except BlockingIOError:
                        finished.wait(.005)

            def controlling_terminal():
                os.setsid()
                if with_tty:
                    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            try:
                process = subprocess.Popen(["bash", str(root / "scripts/warp-notify.sh"), "warp://cli-agent", body],
                                           cwd=root, env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, pass_fds=(slave,), preexec_fn=controlling_terminal)
                reader = threading.Thread(target=drain_terminal)
                reader.start()
                stdout, stderr = process.communicate(timeout=5)
                self.assertEqual(process.returncode, 0, stderr.decode(errors="replace"))
                finished.set()
                reader.join(timeout=2)
                while True:
                    try:
                        chunk = os.read(master, 65536)
                    except BlockingIOError:
                        break
                    if not chunk:
                        break
                    data.extend(chunk)
            finally:
                if "process" in locals() and process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                finished.set()
                if "reader" in locals():
                    reader.join(timeout=2)
                os.close(master)
                os.close(slave)
            self.assertFalse((root / "INJECTED").exists())
            return raw, bytes(data), stdout

    def test_direct_tty_preserves_osc_without_shell_evaluation(self):
        for agent in ("claude", "codex"):
            with self.subTest(agent=agent):
                raw, received, stdout = self.notify(agent, False)
                self.assertEqual(received, raw)
                self.assertEqual(stdout, b"")

    def test_tmux_tty_wraps_dcs_and_doubles_every_escape(self):
        for agent, version in (("codex", None), ("claude", None), ("claude", "2.1.140")):
            with self.subTest(agent=agent, version=version):
                raw, received, stdout = self.notify(agent, True, version)
                self.assertEqual(received, b"\x1bPtmux;" + raw.replace(b"\x1b", b"\x1b\x1b") + b"\x1b\\")
                self.assertEqual(stdout, b"")

    def test_modern_claude_uses_native_raw_sequence_even_with_tty(self):
        raw, received, stdout = self.notify("claude", True, "2.1.273")
        self.assertEqual(received, b"")
        self.assertEqual(json.loads(stdout), {"terminalSequence": raw.decode()})

    def test_unknown_claude_without_tty_falls_back_to_raw_json(self):
        raw, received, stdout = self.notify("claude", True, with_tty=False)
        self.assertEqual(received, b"")
        self.assertEqual(json.loads(stdout), {"terminalSequence": raw.decode()})


if __name__ == "__main__":
    unittest.main()
