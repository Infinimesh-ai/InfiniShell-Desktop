"""实际控制终端测试；复制缓存夹具不等于 Grok 原生安装验收。"""

import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import tempfile
import stat
import subprocess
import time
import shlex
from types import SimpleNamespace
from unittest.mock import patch
import unittest

from run_installed_grok_hook import (DIAGNOSTIC_LIMIT, diagnostic_preload, plain_file,
                                     verify_installed_hook, worker_diagnostics)


NOTIFICATION_PREFIX = b"\x1b]777;notify;warp://cli-agent;"
TERMINAL_OBSERVATION_SECONDS = 2


def terminal_output_summary(output, *, tmux=False):
    prefix, suffix = (b"\x1bPtmux;", b"\x1b\\") if tmux else (NOTIFICATION_PREFIX, b"\x07")
    return json.dumps({"byte_count": len(output), "sha256": hashlib.sha256(output).hexdigest(),
                       "complete": output.startswith(prefix) and output.endswith(suffix)}, sort_keys=True)


def collect_terminal_output(descriptor, *, expect_notification):
    result = bytearray()
    deadline = time.monotonic() + TERMINAL_OBSERVATION_SECONDS

    def fail(reason):
        raise AssertionError(f"{reason}: {terminal_output_summary(result)}") from None

    with selectors.DefaultSelector() as selector:
        selector.register(descriptor, selectors.EVENT_READ)
        while True:
            try:
                chunk = os.read(descriptor, 65536)
            except BlockingIOError:
                # 写端退出不代表 PTY 已完成排队投递；只在完整帧读完后允许结束。
                complete = result.startswith(NOTIFICATION_PREFIX) and result.endswith(b"\x07")
                if complete:
                    return bytes(result)
                # 截止点前已就绪的数据必须先读完，避免调度延迟令负例误报零字节。
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    if expect_notification:
                        fail("terminal_read_deadline")
                    return bytes(result)
                selector.select(timeout=remaining)
                continue
            except OSError as error:
                fail(f"terminal_read_error_{error.errno}")
            if not chunk:
                fail("terminal_read_eof")
            result.extend(chunk)
            if len(result) > 65536:
                fail("terminal_output_limit")
            if not expect_notification:
                fail("unexpected_terminal_output")


def setUpModule():
    if os.name != "posix":
        return
    # 复制后的 macOS 可执行文件首次启动可能受文件属性处理影响。
    # 单独核对冷启动文件属性和协议，再验收已就绪 worker 的真实终端路径；不把预热算作 hook 投递。
    worker = os.environ.get("INFINISHELL_TEST_NOTIFY_WORKER")
    if not worker:
        raise ValueError("INFINISHELL_TEST_NOTIFY_WORKER 必须指向同源码构建的原生 worker")
    path = Path(worker).resolve(strict=True)
    plain_file(path, hashlib.sha256(path.read_bytes()).hexdigest(), executable=True)
    environment = {key: value for key, value in os.environ.items()
                   if key in {"PATH", "HOME", "TMPDIR"}}
    started = time.monotonic()
    result = subprocess.run([str(path), "cli-agent-notify", "--protocol-version"],
                            env=environment, capture_output=True, timeout=15, check=True)
    if result.stdout != b'{"protocol":1,"maxFrameBytes":4096}\n' or result.stderr:
        raise ValueError("原生 worker 冷启动协议不匹配")
    print(f"原生 worker 冷启动协议预检：{(time.monotonic() - started):.3f} 秒；后续只验已就绪路径")


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
        self.assertEqual(result["plugin_version"], "0.1.6")
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
        self.hook_diagnostics = []

    def diagnostic_summary(self):
        return "worker_diagnostics=" + json.dumps(self.hook_diagnostics, ensure_ascii=True, separators=(",", ":"))

    def run_hook(self, extra, hook=None, *, invocation="require(process.argv[1]).main();"):
        # 和 Grok 的 hook 一样真正 setsid 且三条标准流都是管道；先证明 /dev/tty 报 ENXIO。
        check = ("const fs=require('node:fs');try {const fd=fs.openSync('/dev/tty','w');"
                 "fs.closeSync(fd);process.exit(90)} catch(e) {if(e.code!=='ENXIO')process.exit(91)};"
                 + invocation)
        # 匿名诊断文件只传给原 Node；不传入 PTY，不改变 setsid、环境或原调用次数。
        with tempfile.TemporaryFile(mode="w+b", dir=self.root) as diagnostic_file, \
                tempfile.NamedTemporaryFile(mode="w", prefix="hook-diagnostic-", suffix=".cjs",
                                            dir=self.root) as preload:
            preload.write(diagnostic_preload(self.worker, diagnostic_file.fileno()))
            preload.flush()
            try:
                try:
                    result = subprocess.run([str(self.node), "--require", preload.name, "-e", check,
                                             str(hook or self.hook)],
                                            input=self.payload, capture_output=True, start_new_session=True,
                                            env={**self.env, **extra}, timeout=8, check=True,
                                            pass_fds=(diagnostic_file.fileno(),))
                finally:
                    raw = os.pread(diagnostic_file.fileno(), DIAGNOSTIC_LIMIT + 1, 0)
                    self.hook_diagnostics.append(worker_diagnostics(raw))
                self.assertEqual(result.stdout, b"")
                self.assertEqual(result.stderr, b"")
            except (OSError, subprocess.SubprocessError, AssertionError) as error:
                error.add_note(self.diagnostic_summary())
                raise

    def output(self, index, *, expect_notification=False):
        # 非目标终端必须完成同一观察窗口，不能把首次 EAGAIN 当作零字节证据。
        try:
            return collect_terminal_output(self.terminals[index][0], expect_notification=expect_notification)
        except AssertionError as error:
            error.add_note(self.diagnostic_summary())
            raise

    def assert_notification(self, output, tmux=False):
        summary = terminal_output_summary(output, tmux=tmux) + "; " + self.diagnostic_summary()
        if tmux:
            self.assertTrue(output.startswith(b"\x1bPtmux;"), summary)
            self.assertTrue(output.endswith(b"\x1b\\"), summary)
            output = output[7:-2].replace(b"\x1b\x1b", b"\x1b")
        self.assertTrue(output.startswith(NOTIFICATION_PREFIX), summary)
        self.assertTrue(output.endswith(b"\x07"), summary)
        try:
            event = json.loads(output[len(NOTIFICATION_PREFIX):-1])
        except ValueError:
            raise AssertionError(f"terminal_notification_json_invalid: {summary}") from None
        self.assertTrue(isinstance(event, dict), summary)
        for field, expected in (("event", "session_start"), ("session_id", "detached-terminal-test"),
                                ("plugin_version", "0.1.6"), ("agent", "grok")):
            self.assertTrue(event.get(field) == expected, f"terminal_notification_{field}_invalid: {summary}")

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
        self.assert_notification(self.output(0, expect_notification=True))
        self.assertEqual(self.output(1), b"")
        self.assertFalse((self.root / "data").exists())

    def test_ssh_terminal_without_bootstrap_still_uses_real_pty(self):
        self.run_hook({"SSH_TTY": self.terminals[0][2]})
        self.assert_notification(self.output(0, expect_notification=True))
        self.assertEqual(self.output(1), b"")

    def test_guarded_send_requires_real_worker_success_receipt_and_target_pty_frame(self):
        # main 安静降级；另以同一真实发送的返回值核对成功收据，不替代既有 main 用例。
        self.run_hook({"SSH_TTY": self.terminals[0][2]}, invocation=(
            "const hook=require(process.argv[1]);"
            "const notification=hook.makeNotification(hook.normalize("
            "JSON.parse(fs.readFileSync(0,'utf8')),process.env));"
            "if(!notification||hook.sendNotification(notification,process.env)!==true)process.exit(92);"))
        self.assert_notification(self.output(0, expect_notification=True))
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
                                  check=True, capture_output=True, text=True, timeout=15).stdout.strip()
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


class TerminalOutputReadTests(unittest.TestCase):
    def setUp(self):
        # 只模拟读取与时间，不启动 hook、worker 或真实终端。
        self.now = 0
        self.wait_delays = iter(())
        clock = patch.object(time, "monotonic", side_effect=lambda: self.now)
        self.addCleanup(clock.stop)
        clock.start()
        reader = patch.object(os, "read")
        self.addCleanup(reader.stop)
        self.reader = reader.start()
        selector = patch.object(selectors, "DefaultSelector")
        self.addCleanup(selector.stop)
        self.selector = selector.start().return_value.__enter__.return_value

        def wait(timeout):
            elapsed = next(self.wait_delays, timeout)
            self.assertLessEqual(elapsed, timeout)
            self.now += elapsed
            return []

        self.selector.select.side_effect = wait
        self.frame = NOTIFICATION_PREFIX + b'{"event":"session_start"}\x07'

    def test_initial_eagain_waits_for_the_single_notification(self):
        self.wait_delays = iter((.05,))
        self.reader.side_effect = [BlockingIOError(), self.frame, BlockingIOError()]
        self.assertEqual(collect_terminal_output(123, expect_notification=True), self.frame)
        self.assertEqual(self.reader.call_count, 3)
        self.selector.select.assert_called_once()

    def test_split_frame_is_accumulated_across_eagain(self):
        self.wait_delays = iter((.05,))
        self.reader.side_effect = [self.frame[:12], BlockingIOError(), self.frame[12:], BlockingIOError()]
        self.assertEqual(collect_terminal_output(123, expect_notification=True), self.frame)
        self.selector.select.assert_called_once()

    def test_truncated_frame_fails_at_deadline_with_only_a_safe_summary(self):
        truncated = self.frame[:-1]
        self.reader.side_effect = [truncated, BlockingIOError(), BlockingIOError()]
        with self.assertRaises(AssertionError) as failure:
            collect_terminal_output(123, expect_notification=True)
        self.assertEqual(str(failure.exception), "terminal_read_deadline: " + terminal_output_summary(truncated))
        self.assertEqual(self.now, TERMINAL_OBSERVATION_SECONDS)

    def test_missing_notification_fails_after_the_full_observation_window(self):
        self.reader.side_effect = [BlockingIOError(), BlockingIOError()]
        with self.assertRaises(AssertionError) as failure:
            collect_terminal_output(123, expect_notification=True)
        self.assertEqual(str(failure.exception), "terminal_read_deadline: " + terminal_output_summary(b""))
        self.assertEqual(self.now, TERMINAL_OBSERVATION_SECONDS)

    def test_zero_output_requires_the_full_observation_window(self):
        self.reader.side_effect = [BlockingIOError(), BlockingIOError()]
        self.assertEqual(collect_terminal_output(123, expect_notification=False), b"")
        self.assertEqual(self.now, TERMINAL_OBSERVATION_SECONDS)
        self.selector.select.assert_called_once_with(timeout=TERMINAL_OBSERVATION_SECONDS)

    def test_delayed_output_on_a_non_target_terminal_is_rejected(self):
        self.wait_delays = iter((.05,))
        self.reader.side_effect = [BlockingIOError(), self.frame]
        with self.assertRaises(AssertionError) as failure:
            collect_terminal_output(123, expect_notification=False)
        self.assertEqual(str(failure.exception), "unexpected_terminal_output: " + terminal_output_summary(self.frame))
        self.selector.select.assert_called_once()

    def test_ready_output_is_rejected_even_when_scheduling_crosses_the_deadline(self):
        def ready_at_deadline(timeout):
            # 模拟窗口末尾已就绪，但 select 返回调用方时已跨过截止点。
            self.now += timeout + .01
            return [(None, selectors.EVENT_READ)]

        self.selector.select.side_effect = ready_at_deadline
        self.reader.side_effect = [BlockingIOError(), self.frame]
        with self.assertRaises(AssertionError) as failure:
            collect_terminal_output(123, expect_notification=False)
        self.assertEqual(str(failure.exception), "unexpected_terminal_output: " + terminal_output_summary(self.frame))
        self.assertEqual(self.reader.call_count, 2)
        self.selector.select.assert_called_once_with(timeout=TERMINAL_OBSERVATION_SECONDS)


if __name__ == "__main__":
    unittest.main()
