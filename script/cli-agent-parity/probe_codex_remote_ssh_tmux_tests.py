import importlib.util
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import unittest
from unittest import mock


SOURCE = Path(__file__).with_name("probe_codex_remote_ssh_tmux.py")
SPEC = importlib.util.spec_from_file_location("probe_codex_remote_ssh_tmux", SOURCE)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class RemoteCodexSshTmuxTests(unittest.TestCase):
    def setUp(self):
        self.notifier = SOURCE.parents[2] / (
            "app/assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/warp-notify.sh")

    def test_safe_root_rejects_parent_traversal_and_accepts_private_cache(self):
        self.assertIsNotNone(PROBE.SAFE_REMOTE_ROOT.fullmatch(
            "/root/.cache/infinishell-parity-ssh-tmux.example_1"))
        self.assertIsNone(PROBE.SAFE_REMOTE_ROOT.fullmatch("/tmp/probe"))
        for value in (
                "/home/test/.cache/infinishell-parity-ssh-tmux.bad",
                "/root/../root/.cache/infinishell-parity-ssh-tmux.bad",
                "/root/a b/.cache/infinishell-parity-ssh-tmux.bad",
                "/root/a;id/.cache/infinishell-parity-ssh-tmux.bad",
                "/root/$(id)/.cache/infinishell-parity-ssh-tmux.bad",
                "/root/.cache/infinishell-parity-ssh-tmux.bad/../escape"):
            with self.subTest(value=value):
                self.assertIsNone(PROBE.SAFE_REMOTE_ROOT.fullmatch(value))

    def test_remote_command_quotes_every_remote_argument(self):
        ssh = ["ssh", "-o", "BatchMode=yes", "example"]
        command = PROBE.remote_command(
            ssh, "python3", "/root/path with space/probe.py", "$(id)", "a;b")
        self.assertEqual(command[:-1], ssh)
        self.assertEqual(
            command[-1], "python3 '/root/path with space/probe.py' '$(id)' 'a;b'")
        tty_command = PROBE.remote_command(ssh, "printf", "%s", "a b", force_tty=True)
        self.assertEqual(tty_command[:-1], [*ssh[:-1], "-tt", ssh[-1]])
        self.assertEqual(tty_command[-1], "printf %s 'a b'")

    def test_transfer_rejects_unsafe_root_before_remote_or_tar_access(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "file").write_text("payload")
            with mock.patch.object(PROBE.subprocess, "run") as run:
                with self.assertRaises(RuntimeError):
                    PROBE.transfer_tree(
                        ["ssh", "example"], source,
                        "/root/../root/.cache/infinishell-parity-ssh-tmux.bad")
            run.assert_not_called()

    def test_transfer_preflights_resolved_directory_before_tar_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "file").write_text("payload")
            remote_root = "/root/.cache/infinishell-parity-ssh-tmux.safe"
            preflight = subprocess.CompletedProcess(
                [], 0, json.dumps({"root": remote_root}).encode(), b"")
            upload = subprocess.CompletedProcess([], 0, b"", b"")
            with mock.patch.object(
                    PROBE.subprocess, "run", side_effect=[preflight, upload]) as run:
                PROBE.transfer_tree(["ssh", "example"], source, remote_root)
            self.assertEqual(run.call_count, 2)
            self.assertIn("path.resolve(strict=True)", run.call_args_list[0].args[0][-1])
            self.assertEqual(run.call_args_list[1].args[0][:-1], ["ssh", "example"])
            self.assertEqual(
                run.call_args_list[1].args[0][-1],
                "tar -xf - -C /root/.cache/infinishell-parity-ssh-tmux.safe")

    def test_transfer_stops_before_tar_when_remote_root_resolves_elsewhere(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "file").write_text("payload")
            remote_root = "/root/.cache/infinishell-parity-ssh-tmux.safe"
            preflight = subprocess.CompletedProcess(
                [], 0, json.dumps({"root": "/tmp/redirected"}).encode(), b"")
            with mock.patch.object(
                    PROBE.subprocess, "run", return_value=preflight) as run:
                with self.assertRaises(RuntimeError):
                    PROBE.transfer_tree(["ssh", "example"], source, remote_root)
            run.assert_called_once()

    def test_sanitize_removes_raw_bytes_and_private_values(self):
        value = {"ssh_stdout_base64": "secret", "block_output": {"stopReason": "nonce"},
                 "path": "/root/.cache/infinishell-parity-ssh-tmux.test/work",
                 "nested": ["nonce"]}
        result = PROBE.sanitize(value, "/root/.cache/infinishell-parity-ssh-tmux.test",
                                {"direct": "nonce"})
        self.assertNotIn("ssh_stdout_base64", result)
        self.assertNotIn("block_output", result)
        self.assertEqual(result["path"], "<remote-private-root>/work")
        self.assertEqual(result["nested"], ["<probe-token>"])

    def test_write_exclusive_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "receipt.json"
            PROBE.write_exclusive(path, {"passed": True})
            self.assertEqual(json.loads(path.read_text()), {"passed": True})
            with self.assertRaises(FileExistsError):
                PROBE.write_exclusive(path, {"passed": False})

    @unittest.skipUnless(os.name == "posix", "TTY 契约只适用于 POSIX")
    def test_notifier_accepts_strict_ssh_tty_and_rejects_non_terminal(self):
        master, slave = os.openpty()
        try:
            values = os.environ.copy()
            values.update({"WARP_CLI_AGENT_PROTOCOL_VERSION": "1",
                           "WARP_CLIENT_VERSION": "unit-test", "SSH_TTY": os.ttyname(slave)})
            result = subprocess.run(
                ["bash", str(self.notifier), "warp://cli-agent", '{"event":"test"}'],
                env=values, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            ready, _, _ = select.select([master], [], [], 0.2)
            self.assertTrue(ready)
            self.assertIn(b"\x1b]777;notify;warp://cli-agent;{\"event\":\"test\"}\x07",
                          os.read(master, 65536))
        finally:
            os.close(slave)
            os.close(master)
        result = subprocess.run(
            ["bash", str(self.notifier), "warp://cli-agent", '{}'],
            env={**os.environ, "WARP_CLI_AGENT_PROTOCOL_VERSION": "1",
                 "WARP_CLIENT_VERSION": "unit-test", "SSH_TTY": "/dev/null"},
            capture_output=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("infinishell_codex_hook_transport_error: invalid_ssh_tty",
                      result.stderr.decode())

    @unittest.skipUnless(os.name == "posix", "TTY 契约只适用于 POSIX")
    def test_notifier_queries_current_tmux_pane_and_wraps_dcs(self):
        with tempfile.TemporaryDirectory() as temporary:
            master, slave = os.openpty()
            try:
                fake_tmux = Path(temporary) / "tmux"
                fake_tmux.write_text("#!/bin/sh\n[ \"$1 $2 $3 $4\" = \"display-message -p -t %7\" ] || exit 9\n"
                                     "[ \"$5\" = '#{pane_tty}' ] || exit 8\nprintf '%s\\n' \"$FAKE_TTY\"\n")
                fake_tmux.chmod(0o755)
                values = os.environ.copy()
                values.update({"WARP_CLI_AGENT_PROTOCOL_VERSION": "1",
                               "WARP_CLIENT_VERSION": "unit-test", "TMUX": "/tmp/socket,1,0",
                               "TMUX_PANE": "%7", "FAKE_TTY": os.ttyname(slave),
                               "PATH": temporary + os.pathsep + values["PATH"]})
                result = subprocess.run(
                    ["bash", str(self.notifier), "warp://cli-agent", '{}'], env=values,
                    capture_output=True, timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                ready, _, _ = select.select([master], [], [], 0.2)
                self.assertTrue(ready)
                output = os.read(master, 65536)
                self.assertTrue(output.startswith(b"\x1bPtmux;"))
                self.assertIn(b"\x1b\x1b]777;notify", output)
            finally:
                os.close(slave)
                os.close(master)


if __name__ == "__main__":
    unittest.main()
