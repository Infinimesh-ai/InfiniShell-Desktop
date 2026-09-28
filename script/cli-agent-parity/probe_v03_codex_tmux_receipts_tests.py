#!/usr/bin/env python3
"""V03 双端原始收据必须同时证明内层发送和外层未收到。"""

import base64
import hashlib
import json
from pathlib import Path
import unittest

import probe_v03_codex_tmux_receipts as probe


def osc(event, session="native-session", turn=None):
    body = {"v": 1, "agent": "codex", "event": event, "session_id": session,
        "cwd": "/root/.cache/infinishell-parity-ssh-tmux.test/work/tmux-off"}
    if turn is not None:
        body["turn_id"] = turn
    raw = b"\x1b]777;notify;warp://cli-agent;" + json.dumps(body).encode() + b"\x07"
    return b"\x1bPtmux;" + raw.replace(b"\x1b", b"\x1b\x1b") + b"\x1b\\"


class V03CodexTmuxReceiptTests(unittest.TestCase):
    def setUp(self):
        self.inner = osc("session_start") + osc("prompt_submit", turn="native-turn")
        self.outer = b"Codex native TUI completed the hook\r\n"
        self.case = {"ssh_stdout_base64": base64.b64encode(self.outer).decode(),
            "ssh_stdout_sha256": hashlib.sha256(self.outer).hexdigest(),
            "ssh_stdout_bytes": len(self.outer)}
        self.native = {"SessionStart": {"environment": {"TMUX_PANE": "%7"}},
            "UserPromptSubmit": {"input": {"session_id": "native-session",
                "turn_id": "native-turn",
                "cwd": "/root/.cache/infinishell-parity-ssh-tmux.test/work/tmux-off"}},
            "native-start": {"tty": "/dev/pts/7"}}
        self.capture = {"pane": "%7", "pane_tty": "/dev/pts/7",
            "allow_passthrough": "off", "installed_before_codex_start": True}

    def test_current_fixed_codex_package_binding(self):
        repository = Path(__file__).resolve().parents[2]
        package = json.loads((repository / "script/cli-agent-parity/codex_0156_package_manifest.json").read_text())[
            "packages"]["linux-x64"]
        probe.verify_fixed_codex_hashes(repository, package["files"]["bin/codex"][1],
            package["sha256"])
        self.assertEqual(probe.CODEX_VERSION, "0.156.1")
        self.assertEqual(probe.CODEX_RELATIVE, "codex/runtime-0.156.1-linux-x64/bin/codex")
        with self.assertRaises(RuntimeError):
            probe.verify_fixed_codex_hashes(repository, "0" * 64, package["sha256"])
        with self.assertRaises(RuntimeError):
            probe.verify_fixed_codex_hashes(repository, package["files"]["bin/codex"][1], "0" * 64)

    def test_remote_instrumentation_compiles_and_starts_capture_before_codex(self):
        source = probe.remote_source()
        compile(source, "remote_driver.py", "exec")
        self.assertLess(source.index("'pipe-pane','-O'"), source.index(
            "process=subprocess.Popen([str(codex),'--no-alt-screen'"))
        self.assertIn("option!='off'", source)
        self.assertIn("'inner-pane.raw'", source)

    def test_two_native_hook_events_and_outer_zero_pass(self):
        self.assertEqual([value["event"] for value in probe.inner_notifications(self.inner)],
            ["session_start", "prompt_submit"])
        receipt = probe.validate_inner_outer(self.case, self.native, self.capture, self.inner)
        self.assertEqual(receipt["native_hook_records"], 2)
        self.assertEqual(receipt["outer_notification_count"], 0)
        self.assertTrue(receipt["same_native_session_and_turn"])

    def test_untriggered_or_wrong_native_hook_does_not_count_as_blocked(self):
        for inner in (b"", osc("session_start"),
                      osc("session_start") + osc("prompt_submit", turn="different"),
                      osc("session_start", session="other") + osc("prompt_submit", turn="native-turn")):
            with self.subTest(inner=inner):
                with self.assertRaises(RuntimeError):
                    probe.validate_inner_outer(self.case, self.native, self.capture, inner)

    def test_outer_notification_and_mismatched_raw_digest_fail(self):
        outer = b"leaked " + osc("prompt_submit", turn="native-turn")
        self.case["ssh_stdout_base64"] = base64.b64encode(outer).decode()
        self.case["ssh_stdout_sha256"] = hashlib.sha256(outer).hexdigest()
        self.case["ssh_stdout_bytes"] = len(outer)
        with self.assertRaises(RuntimeError):
            probe.validate_inner_outer(self.case, self.native, self.capture, self.inner)
        self.case["ssh_stdout_sha256"] = "0" * 64
        with self.assertRaises(RuntimeError):
            probe.validate_inner_outer(self.case, self.native, self.capture, self.inner)

    def test_incorrect_tmux_option_or_late_capture_fails(self):
        for mutation in ({"allow_passthrough": "on"},
                         {"installed_before_codex_start": False}, {"pane_tty": "/dev/pts/9"}):
            with self.subTest(mutation=mutation):
                with self.assertRaises(RuntimeError):
                    probe.validate_inner_outer(self.case, self.native,
                        {**self.capture, **mutation}, self.inner)

    def test_safe_receipt_excludes_raw_stream_and_native_ids(self):
        report = {"cases": [{"ssh_stdout_base64": self.case["ssh_stdout_base64"],
            "inner_pane_base64": base64.b64encode(self.inner).decode(),
            "native": self.native, "notifications": [{"session_id": "native-session"}],
            "native_session_id": "native-session", "native_turn_id": "native-turn",
            "dual_end_receipt": {"inner_sha256": hashlib.sha256(self.inner).hexdigest()}}],
            "authorization_rpc": [{"secret": "private"}], "native_install": "private"}
        safe = probe.safe_report(report, "/root/.cache/infinishell-parity-ssh-tmux.test", {})
        rendered = json.dumps(safe)
        for private in ("native-session", "native-turn", "private", "inner_pane_base64",
                        "ssh_stdout_base64"):
            self.assertNotIn(private, rendered)
        self.assertFalse(safe["safe_receipt_contains_raw_terminal_bytes"])


if __name__ == "__main__":
    unittest.main()
