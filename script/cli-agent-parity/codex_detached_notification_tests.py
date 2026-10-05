#!/usr/bin/env python3
"""用隔离的真实 PTY 验证 Codex 脱离会话后的通知，不启动真实 CLI。"""

import errno
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import unittest


ROOT = Path(__file__).resolve().parents[2]
NOTIFIER = ROOT / "app/assets/bundled/cli-agent-plugins/codex/scripts/warp-notify.sh"
BODY = '{"v":1,"agent":"codex","event":"stop","session_id":"fixture"}'
EXPECTED = ("\x1b]777;notify;warp://cli-agent;" + BODY + "\x07").encode()

# 系统 Bash 副本会触发 macOS 的启动约束；自行构建无特权包装器保留真实进程名。
PROCESS_WRAPPER = r'''
#include <errno.h>
#include <fcntl.h>
#include <sys/wait.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 2) return 64;
    pid_t child = fork();
    if (child < 0) return 71;
    if (child == 0) {
        int input = open("/dev/null", O_RDONLY);
        if (input < 0 || dup2(input, STDIN_FILENO) < 0) _exit(71);
        if (input != STDIN_FILENO) close(input);
        execv(argv[1], &argv[1]);
        _exit(126);
    }
    int status;
    while (waitpid(child, &status, 0) < 0) {
        if (errno != EINTR) return 71;
    }
    if (WIFEXITED(status)) return WEXITSTATUS(status);
    if (WIFSIGNALED(status)) return 128 + WTERMSIG(status);
    return 71;
}
'''

# 固定旧的六层 ps 实现作为负对照，不依赖浅克隆中可能不存在的 Git 历史。
LEGACY_DISCOVERY = r'''open_ancestor_terminal() {
    local pid="$PPID" command_name tty_name candidate attempt
    for attempt in 1 2 3 4 5 6; do
        case "$pid" in
            ''|*[!0-9]*|0|1) return 1 ;;
        esac
        command_name=$(ps -o comm= -p "$pid" 2>/dev/null) || return 1
        command_name="${command_name##*/}"
        if [ "$command_name" = "codex" ] || [[ "$command_name" == codex-* ]]; then
            tty_name=$(ps -o tty= -p "$pid" 2>/dev/null) || return 1
            tty_name="${tty_name//[[:space:]]/}"
            case "$tty_name" in
                ''|'??'|'?') ;;
                *)
                    candidate="/dev/$tty_name"
                    open_terminal "$candidate" && return 0
                    ;;
            esac
        fi
        pid=$(ps -o ppid= -p "$pid" 2>/dev/null) || return 1
        pid="${pid//[[:space:]]/}"
    done
    return 1
}
'''


def fixture_command(executable, mode, config):
    return [str(executable), sys.executable, "-B", str(Path(__file__).resolve()),
            mode, json.dumps(config)]


def collect_process(process, timeout):
    try:
        return process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        # 只结束本夹具创建且持有句柄的独立进程组。
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=3)
        raise


def run_chain(config):
    tty_fd = config["tty_fd"]
    if tty_fd > 2:
        os.close(tty_fd)
    if config["inner_without_terminal"]:
        inner = {**config, "inner_without_terminal": False, "tty_fd": -1}
        process = subprocess.Popen(
            fixture_command(config["codex"], "--chain", inner),
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True,
        )
        stdout, stderr = collect_process(process, 8)
        sys.stdout.buffer.write(stdout)
        sys.stderr.buffer.write(stderr)
        return process.returncode

    # 每个父进程等待自己的子进程，确保包装器退出并被回收。
    for _ in range(config["depth"]):
        child = os.fork()
        if child:
            _, status = os.waitpid(child, 0)
            return os.waitstatus_to_exitcode(status)

    process = subprocess.Popen(
        ["/bin/bash", config["notifier"], "warp://cli-agent", BODY],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        start_new_session=config["detached"],
    )
    try:
        stdout, stderr = process.communicate(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate(timeout=3)
        raise
    print(json.dumps({"returncode": process.returncode,
                      "stdout": stdout.decode(), "stderr": stderr.decode()}))
    return 0


def run_owner(config):
    import fcntl
    import termios

    def attach_terminal():
        os.setsid()
        if config["controlling_terminal"]:
            fcntl.ioctl(config["tty_fd"], termios.TIOCSCTTY, 0)

    process = subprocess.Popen(
        fixture_command(config["owner"], "--chain", config),
        stdin=config["tty_fd"] if config["terminal_stdin"] else subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        pass_fds=(config["tty_fd"],), preexec_fn=attach_terminal,
    )
    stdout, stderr = collect_process(process, 12)
    sys.stdout.buffer.write(stdout)
    sys.stderr.buffer.write(stderr)
    return process.returncode


@unittest.skipUnless(sys.platform in ("darwin", "linux"), "需要 macOS 或 Linux 真实 PTY")
class CodexDetachedNotificationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="codex-detached-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.scripts = self.directory / "scripts"
        self.scripts.mkdir()
        self.notifier = self.scripts / "warp-notify.sh"
        self.notifier.write_bytes(NOTIFIER.read_bytes())
        # 只替换协议广告门槛；实际通知传输使用待测资源。
        (self.scripts / "should-use-structured.sh").write_text(
            "should_use_structured() { return 0; }\n", encoding="utf-8")
        self.codex = self.directory / "codex"
        self.host_shell = self.directory / "host-shell"
        # 复用已部署编译器；产物和编译临时文件均沿用调用方的受控 TMPDIR。
        compiler = shutil.which("cc") or shutil.which("clang")
        self.assertIsNotNone(compiler, "PTY 夹具需要已部署的 C 编译器")
        subprocess.run([compiler, "-x", "c", "-o", str(self.codex), "-"],
                       input=PROCESS_WRAPPER.encode(), capture_output=True,
                       check=True, timeout=20)
        shutil.copyfile(self.codex, self.host_shell)
        self.codex.chmod(0o700)
        self.host_shell.chmod(0o700)
        self.environment = {key: value for key, value in os.environ.items()
                            if key not in {"TMUX", "TMUX_PANE", "SSH_TTY", "SSH_CONNECTION",
                                           "SSH_CLIENT", "BASH_ENV", "ENV"}}

    def use_old_discovery(self):
        text = self.notifier.read_text(encoding="utf-8")
        start = text.index("open_named_terminal() {")
        end = text.index('case "$(uname -s)" in', start)
        self.notifier.write_text(text[:start] + LEGACY_DISCOVERY + text[end:], encoding="utf-8")

    def disable_ps(self):
        commands = self.directory / "commands"
        commands.mkdir()
        script = commands / "ps"
        script.write_text('#!/bin/bash\nprintf called >> "$TEST_PS_CALLS"\nexit 127\n',
                          encoding="utf-8")
        script.chmod(0o700)
        self.ps_calls = self.directory / "ps.calls"
        self.environment.update(PATH=str(commands) + os.pathsep + os.environ["PATH"],
                                TEST_PS_CALLS=str(self.ps_calls))

    def notify(self, *, depth=0, detached=True, terminal=True, terminal_stdin=True,
               codex_owner=True, inner_without_terminal=False):
        import pty
        import tty

        master, slave = pty.openpty()
        tty.setraw(slave)
        os.set_blocking(master, False)
        config = {"tty_fd": slave, "depth": depth, "detached": detached,
                  "controlling_terminal": terminal, "terminal_stdin": terminal_stdin,
                  "codex": str(self.codex),
                  "owner": str(self.codex if codex_owner else self.host_shell),
                  "inner_without_terminal": inner_without_terminal,
                  "notifier": str(self.notifier)}
        data = bytearray()
        finished = threading.Event()
        reader_errors = []

        def drain():
            # macOS 必须在进程退出前持续排空 PTY，避免退出时等待终端输出。
            while not finished.is_set():
                try:
                    chunk = os.read(master, 65536)
                    if not chunk:
                        return
                    data.extend(chunk)
                except BlockingIOError:
                    finished.wait(0.005)
                except OSError as error:
                    if error.errno != errno.EIO:
                        reader_errors.append(error)
                    return

        # 外层 Codex 无控制终端且标准流全为管道，阻止搜索越过夹具碰到宿主终端。
        process = None
        reader = threading.Thread(target=drain)
        try:
            process = subprocess.Popen(
                fixture_command(self.codex, "--owner", config),
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                pass_fds=(slave,), start_new_session=True, env=self.environment,
            )
            reader.start()
            stdout, stderr = collect_process(process, 18)
            self.assertEqual(process.returncode, 0, stderr.decode(errors="replace"))
            result = json.loads(stdout)
        finally:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate(timeout=3)
            finished.set()
            if reader.ident is not None:
                reader.join(timeout=2)
            while True:
                try:
                    chunk = os.read(master, 65536)
                    if not chunk:
                        break
                    data.extend(chunk)
                except OSError as error:
                    if error.errno in (errno.EAGAIN, errno.EIO):
                        break
                    raise
            os.close(master)
            os.close(slave)
        self.assertEqual(reader_errors, [])
        return result, bytes(data)

    def assert_delivered(self, result, output):
        self.assertEqual(result, {"returncode": 0, "stdout": "", "stderr": ""})
        self.assertEqual(output, EXPECTED)

    def assert_rejected(self, result, output, code="terminal_not_found"):
        self.assertEqual(result["returncode"], 1)
        self.assertEqual(result["stdout"], "")
        self.assertEqual(result["stderr"].strip(),
                         f"infinishell_codex_hook_transport_error: {code}")
        self.assertEqual(output, b"")

    def test_inherited_controlling_terminal(self):
        self.assert_delivered(*self.notify(detached=False))

    def test_setsid_detached_hook_reaches_codex_terminal(self):
        self.assert_delivered(*self.notify())

    def test_detached_hook_traverses_more_than_six_ancestors(self):
        self.assert_delivered(*self.notify(depth=9))

    def test_old_six_ancestor_implementation_rejects_deep_hook(self):
        self.use_old_discovery()
        self.assert_rejected(*self.notify(depth=9))

    @unittest.skipUnless(sys.platform == "linux", "Linux /proc 标准 fd 回退")
    def test_linux_without_ps_uses_codex_terminal_fd(self):
        self.disable_ps()
        self.assert_delivered(*self.notify(terminal=False))
        self.assertFalse(self.ps_calls.exists())

    @unittest.skipUnless(sys.platform == "linux", "旧实现对 Linux ps 的依赖负对照")
    def test_old_implementation_fails_without_ps(self):
        self.use_old_discovery()
        self.disable_ps()
        self.assert_rejected(*self.notify(terminal=False))
        self.assertEqual(self.ps_calls.read_text(), "called")

    def test_controlling_terminal_with_captured_standard_streams(self):
        self.assert_delivered(*self.notify(terminal_stdin=False))

    def test_does_not_use_unrelated_ancestor_terminal(self):
        self.assert_rejected(*self.notify(codex_owner=False))

    def test_nearest_codex_without_terminal_blocks_outer_codex(self):
        self.assert_rejected(*self.notify(inner_without_terminal=True))

    def test_no_terminal_never_writes_stdout(self):
        self.assert_rejected(*self.notify(terminal=False, terminal_stdin=False))

    def test_ancestor_search_is_bounded_to_32(self):
        self.assert_rejected(*self.notify(depth=33))

    def test_invalid_ssh_device_does_not_fall_back_or_write_stdout(self):
        regular_file = self.directory / "ordinary-file"
        regular_file.write_bytes(b"unchanged")
        linked_terminal = self.directory / "linked-terminal"
        linked_terminal.symlink_to("/dev/tty")
        for candidate in ("/dev/null", "/dev/fd/1", "/dev/tty-infinishell-missing",
                          str(regular_file), str(linked_terminal)):
            with self.subTest(candidate=candidate):
                self.environment["SSH_TTY"] = candidate
                self.assert_rejected(*self.notify(), code="invalid_ssh_tty")
        self.assertEqual(regular_file.read_bytes(), b"unchanged")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--owner":
        raise SystemExit(run_owner(json.loads(sys.argv[2])))
    if len(sys.argv) == 3 and sys.argv[1] == "--chain":
        raise SystemExit(run_chain(json.loads(sys.argv[2])))
    unittest.main()
