import base64
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import probe_codex_ssh_tmux_reconnect as probe


class ReconnectProbeTests(unittest.TestCase):
    def notification(self, event, session="session", turn=None):
        value = {"agent": "codex", "event": event, "session_id": session}
        if turn is not None:
            value["turn_id"] = turn
        return value

    def test_detached_status_requires_no_client_and_live_pane(self):
        status = {"session_exit": 0, "sessions": ["reconnect-explicit-detach|0"],
                  "pane_exit": 0, "panes": ["%0|0|12|codex|/dev/pts/1"],
                  "client_exit": 0, "clients": []}
        self.assertTrue(probe.status_is_detached_and_alive(status, "explicit-detach"))
        status["clients"] = ["/dev/pts/2"]
        self.assertFalse(probe.status_is_detached_and_alive(status, "explicit-detach"))

    def test_reconnect_uses_the_same_strict_remote_root_contract(self):
        probe.validate_remote_root("/root/.cache/infinishell-parity-ssh-tmux.reconnect")
        for value in (
                "/tmp/infinishell-parity-ssh-tmux.reconnect",
                "/root/../root/.cache/infinishell-parity-ssh-tmux.reconnect",
                "/root/x;id/.cache/infinishell-parity-ssh-tmux.reconnect"):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                probe.validate_remote_root(value)

    def test_notification_parser_ignores_non_codex_and_invalid_json(self):
        raw = (b"\x1b]777;notify;warp://cli-agent;not-json\x07" +
               b"\x1b]777;notify;warp://cli-agent;{\"agent\":\"claude\"}\x07" +
               b"\x1b]777;notify;warp://cli-agent;{\"agent\":\"codex\","
               b"\"event\":\"session_start\"}\x07")
        self.assertEqual(probe.notification_values(raw),
                         [{"agent": "codex", "event": "session_start"}])

    def test_product_input_keeps_complete_initial_ssh_stream(self):
        first = b"initial"
        report = {"source_commit": "abc", "source_tree_dirty": True,
                  "host_alias": "bwh-2t", "remote_inputs": {"codex_version": "codex-cli 0.155.1"},
                  "cases": [{"method": "abrupt-disconnect", "native_session_id": "session",
                             "native_turn_id": "turn", "initial": {
                                 "ssh_stdout_base64": base64.b64encode(first).decode()},
                             "reattach": {
                                 "ssh_stdout_base64": base64.b64encode(b"reattach").decode()},
                             "initial_native_turn_id": "initial-turn"}]}
        value = probe.build_product_input(report)
        case = value["cases"][0]
        self.assertEqual(base64.b64decode(case["ssh_stdout_base64"]), first)
        self.assertEqual(case["ssh_stdout_sha256"], hashlib.sha256(first).hexdigest())
        self.assertEqual(case["native_turn_id"], "initial-turn")
        self.assertTrue(value["remote_transport_passed"])

    def test_reattach_product_input_selects_real_session_start_and_complete_reattach(self):
        session = "session"
        start = (b"before\x1b]777;notify;warp://cli-agent;"
                 b"{\"agent\":\"codex\",\"event\":\"session_start\","
                 b"\"session_id\":\"session\"}\x07after")
        reattach = (b"redraw\x1b]777;notify;warp://cli-agent;"
                    b"{\"agent\":\"codex\",\"event\":\"prompt_submit\","
                    b"\"session_id\":\"session\",\"turn_id\":\"second\"}\x07")
        report = {"source_commit": "abc", "source_tree_dirty": True,
                  "host_alias": "bwh-2t", "remote_inputs": {"codex_version": "codex-cli 0.155.1"},
                  "cases": [{"method": "explicit-detach", "native_session_id": session,
                             "native_turn_id": "second", "initial": {
                                 "ssh_stdout_base64": base64.b64encode(start).decode()},
                             "reattach": {
                                 "ssh_stdout_base64": base64.b64encode(reattach).decode()}}]}
        value = probe.build_reattach_product_input(report)
        raw = base64.b64decode(value["cases"][0]["ssh_stdout_base64"])
        self.assertNotIn(b"before", raw)
        self.assertTrue(raw.endswith(reattach))
        self.assertFalse(value["complete_live_product_stream"])
        self.assertTrue(value["offline_product_parser_only"])

    def test_cli_writes_consumable_initial_and_reattach_product_inputs(self):
        remote_root = "/root/.cache/infinishell-parity-ssh-tmux.cli"
        methods = ("explicit-detach", "abrupt-disconnect")

        def stream(event, session, turn=None):
            value = {"agent": "codex", "event": event, "session_id": session}
            if turn is not None:
                value["turn_id"] = turn
            return (b"\x1b]777;notify;warp://cli-agent;" +
                    json.dumps(value).encode() + b"\x07")

        captures = []
        for method in methods:
            session = "session-" + method
            captures.extend([
                {"ssh_stdout_base64": base64.b64encode(
                    b"initial-prefix" + stream("session_start", session) +
                    stream("prompt_submit", session, "initial-" + method)).decode()},
                {"ssh_stdout_base64": base64.b64encode(
                    b"reattach-prefix" +
                    stream("prompt_submit", session, "reattach-" + method)).decode()},
            ])

        def validate(case, reports_before, reports, token):
            del reports_before, reports, token
            method = case["method"]
            case.update(
                native_session_id="session-" + method,
                initial_native_turn_id="initial-" + method,
                native_turn_id="reattach-" + method,
                tmux_survived_without_clients=True,
                passed=True,
                old_session_start_replayed_on_reattach=False,
            )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({
                "passed": True,
                "host_alias": "example",
                "remote_inputs": {"root": remote_root,
                                  "codex_version": "codex-cli 0.155.1"},
                "official_codex_hook_control_tty_compatible": True,
                "cases": [{"mode": mode, "native": {"UserPromptSubmit": {
                    "block_output": {"stopReason": "token-" + mode}}}}
                          for mode in ("direct", "tmux-on", "tmux-off")],
            }))
            private = root / "private.json"
            safe = root / "safe.json"
            initial_product = root / "initial-product.json"
            reattach_product = root / "reattach-product.json"
            arguments = [
                "probe_codex_ssh_tmux_reconnect.py",
                "--host", "example",
                "--remote-root", remote_root,
                "--baseline-private", str(baseline),
                "--private-output", str(private),
                "--safe-output", str(safe),
                "--product-private-output", str(initial_product),
                "--reattach-product-private-output", str(reattach_product),
            ]
            with mock.patch.object(sys, "argv", arguments), \
                    mock.patch.object(probe.subprocess, "check_output",
                                      side_effect=["commit\n", ""]), \
                    mock.patch.object(probe, "ssh_base", return_value=["ssh", "example"]), \
                    mock.patch.object(probe, "transfer_tree"), \
                    mock.patch.object(probe, "remote_json", return_value={}), \
                    mock.patch.object(probe, "capture", side_effect=captures), \
                    mock.patch.object(probe, "validate_case", side_effect=validate), \
                    mock.patch.object(
                        probe.subprocess, "run",
                        return_value=probe.subprocess.CompletedProcess([], 0, b"", b"")), \
                    mock.patch("builtins.print"):
                probe.main()

            initial_value = json.loads(initial_product.read_text())
            reattach_value = json.loads(reattach_product.read_text())
            self.assertEqual(len(initial_value["cases"]), 2)
            self.assertEqual(len(reattach_value["cases"]), 2)
            self.assertTrue(reattach_value["offline_product_parser_only"])
            for index, method in enumerate(methods):
                initial_raw = base64.b64decode(
                    initial_value["cases"][index]["ssh_stdout_base64"])
                reattach_raw = base64.b64decode(
                    reattach_value["cases"][index]["ssh_stdout_base64"])
                self.assertIn(b"initial-prefix", initial_raw)
                self.assertNotIn(b"initial-prefix", reattach_raw)
                self.assertIn(b"reattach-prefix", reattach_raw)
                self.assertEqual(
                    reattach_value["cases"][index]["native_turn_id"],
                    "reattach-" + method)


if __name__ == "__main__":
    unittest.main()
