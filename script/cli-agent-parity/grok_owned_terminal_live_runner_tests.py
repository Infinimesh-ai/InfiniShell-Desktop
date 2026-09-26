#!/usr/bin/env python3
"""隔离验收清理不得因 PID 复用或无法核实身份而终止其他进程。"""

import unittest
from unittest.mock import Mock, patch

import run_grok_owned_terminal_live as runner


class OwnedCleanupTests(unittest.TestCase):
    def setUp(self):
        self.expected = {"pid": 10, "unique_id": 20, "pid_version": 3, "resource_cid": 4}

    def test_reused_pid_is_not_signalled(self):
        current = dict(self.expected, unique_id=21)
        with patch.object(runner, "process_identity", return_value=current), patch.object(runner.os, "kill") as kill:
            self.assertTrue(runner.stop_owned(self.expected))
            kill.assert_not_called()

    def test_unknown_identity_is_not_signalled(self):
        with patch.object(runner, "process_identity", side_effect=RuntimeError("unknown")), patch.object(runner.os, "kill") as kill:
            with self.assertRaises(RuntimeError):
                runner.stop_owned(self.expected)
            kill.assert_not_called()

    def test_exec_does_not_make_original_process_look_exited(self):
        current = dict(self.expected, pid_version=5)
        with patch.object(runner, "process_identity", side_effect=[current, None]), patch.object(runner.os, "kill") as kill:
            self.assertTrue(runner.stop_owned(self.expected))
            kill.assert_called_once_with(10, runner.signal.SIGTERM)

    def test_pid_reuse_during_wait_does_not_receive_sigkill(self):
        reused = dict(self.expected, unique_id=22)
        with patch.object(runner, "process_identity", side_effect=[self.expected, reused]), patch.object(runner.os, "kill") as kill:
            self.assertTrue(runner.stop_owned(self.expected))
            kill.assert_called_once_with(10, runner.signal.SIGTERM)

    def test_master_closes_before_shell_wait(self):
        shell = Mock()
        order = []
        shell.wait.side_effect = lambda **kwargs: order.append("wait")
        with patch.object(runner.os, "close", side_effect=lambda fd: order.append("close")):
            self.assertTrue(runner.close_pty_and_reap_shell(8, shell))
        self.assertEqual(order, ["close", "wait"])
        shell.wait.assert_called_once_with(timeout=4)
        shell.kill.assert_not_called()

    def test_shell_timeout_is_bounded_after_kill(self):
        shell = Mock()
        shell.wait.side_effect = runner.subprocess.TimeoutExpired("shell", 4)
        with patch.object(runner.os, "close") as close:
            self.assertFalse(runner.close_pty_and_reap_shell(8, shell))
            close.assert_called_once_with(8)
        self.assertEqual(shell.wait.call_count, 2)
        self.assertTrue(all(call.kwargs == {"timeout": 4} for call in shell.wait.call_args_list))
        shell.kill.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
