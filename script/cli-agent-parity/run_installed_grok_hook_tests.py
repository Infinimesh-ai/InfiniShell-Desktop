"""同一次 hook 原生调用的有限诊断协议；不执行 Node 或通知 worker。"""

import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

from run_installed_grok_hook import DIAGNOSTIC_LIMIT, hook_failure, verify_installed_hook, worker_diagnostics


def row(stage, **values):
    return {"stage": stage, "elapsed_ms": 0, "exit_code": None, "signal": None,
            "stdout_bytes": 0, "stderr_bytes": 0, "error_code": None, **values}


def encode(*rows):
    return ("\n".join(json.dumps(value) for value in rows) + "\n").encode()


class WorkerDiagnosticTests(unittest.TestCase):
    def test_native_terminal_error_is_preserved_without_stderr_text(self):
        rows = [row("preload"), row("protocol", exit_code=0, elapsed_ms=0.5, stdout_bytes=41,
                                   stderr_bytes=None),
                row("send", exit_code=1, elapsed_ms=2.5, stdout_bytes=0, stderr_bytes=47,
                    error_code="cli_agent_notify_terminal_unavailable")]
        self.assertEqual(worker_diagnostics(encode(*rows)), {"status": "captured", "events": rows})

    def test_outer_send_deadline_preserves_its_fixed_phase(self):
        rows = [row("preload"), row("protocol", exit_code=0, stdout_bytes=41, stderr_bytes=None),
                row("send", exit_code=1, error_code="cli_agent_notify_send_timeout_lock_open")]
        self.assertEqual(worker_diagnostics(encode(*rows))["events"][2]["error_code"],
                         "cli_agent_notify_send_timeout_lock_open")

    def test_successful_exec_keeps_unobservable_stderr_count_unknown(self):
        rows = [row("preload"), row("protocol", exit_code=0, stdout_bytes=41, stderr_bytes=None),
                row("send", exit_code=0, stderr_bytes=None)]
        observed = worker_diagnostics(encode(*rows))
        self.assertIsNone(observed["events"][2]["stderr_bytes"])

    def test_preload_only_differs_from_missing_diagnostic(self):
        self.assertEqual(worker_diagnostics(b""), {"status": "missing"})
        self.assertEqual(worker_diagnostics(encode(row("preload")))["events"], [row("preload")])

    def test_unknown_fields_and_text_are_never_exported_in_failure(self):
        for key in ("payload", "path", "environment", "stderr"):
            with self.subTest(key=key):
                value = row("preload", **{key: "private-sentinel"})
                error = hook_failure("hook_main_notification_missing:tty_bytes=0", encode(value))
                self.assertNotIn("private-sentinel", str(error))
                self.assertEqual(str(error), 'hook_main_notification_missing:tty_bytes=0; '
                                            'worker_diagnostics={"status":"invalid"}')

    def test_unknown_error_signal_and_non_scalar_values_are_rejected(self):
        for key, value in (("error_code", "private-sentinel"), ("signal", "private-sentinel"),
                           ("error_code", "cli_agent_notify_send_timeout_private_sentinel"),
                           ("signal", []), ("error_code", {}), ("stdout_bytes", "private-sentinel")):
            with self.subTest(key=key, value=value):
                result = worker_diagnostics(encode(row("preload"), row("protocol", **{key: value})))
                self.assertEqual(result, {"status": "invalid"})

    def test_numeric_budgets_reject_nan_infinity_large_integer_bool_and_negative(self):
        for key, value in (("elapsed_ms", float("nan")), ("elapsed_ms", float("inf")),
                           ("elapsed_ms", 10 ** 600), ("elapsed_ms", True), ("elapsed_ms", -1),
                           ("stdout_bytes", True), ("stderr_bytes", 1048577), ("exit_code", -1)):
            with self.subTest(key=key, value=value):
                self.assertEqual(worker_diagnostics(encode(row("preload", **{key: value}))),
                                 {"status": "invalid"})

    def test_stage_order_duplicates_and_send_after_failed_protocol_are_rejected(self):
        cases = [[row("send")], [row("preload"), row("send")],
                 [row("preload"), row("protocol"), row("protocol")],
                 [row("preload"), row("protocol", exit_code=1, error_code="unknown"), row("send")],
                 [row("preload")] * 4]
        for values in cases:
            with self.subTest(values=values):
                self.assertEqual(worker_diagnostics(encode(*values)), {"status": "invalid"})

    def test_truncated_invalid_utf8_json_and_oversized_data_do_not_escape(self):
        for raw in (b"\xff\n", b"{bad json}\n", encode(row("preload"))[:-1], b"[]\n"):
            with self.subTest(raw=raw):
                self.assertEqual(worker_diagnostics(raw), {"status": "invalid"})
        self.assertEqual(worker_diagnostics(b"x" * (DIAGNOSTIC_LIMIT + 1)), {"status": "over_budget"})

    @unittest.skipUnless(os.name == "posix", "控制终端 helper 仅适用于 Unix")
    def test_preload_setup_preserves_worker_environment_and_cleans_up_failed_spawn(self):
        with tempfile.TemporaryDirectory(prefix="hook-diagnostic-test-") as temporary:
            root = Path(temporary).resolve()
            for name in ("home", "grok", "tmp"):
                (root / name).mkdir(mode=0o700)
            files = []
            for name in ("node", "notify.cjs", "worker"):
                path = root / name
                path.write_bytes(b"synthetic-no-execution")
                path.chmod(0o700)
                files.append((path, hashlib.sha256(path.read_bytes()).hexdigest()))
            environment = {"HOME": str(root / "home"), "GROK_HOME": str(root / "grok"),
                           "TMPDIR": str(root / "tmp"), "PATH": "/usr/bin:/bin"}
            before_environment = copy.deepcopy(environment)
            before_files = set(root.iterdir())
            observed = {}

            def fail_spawn(argv, **options):
                self.assertEqual(argv[0], str(files[0][0]))
                self.assertEqual(argv[1], "--require")
                self.assertEqual(argv[3], str(files[1][0]))
                preload = Path(argv[2])
                self.assertEqual(preload.parent, root)
                self.assertEqual(stat.S_IMODE(preload.stat().st_mode), 0o600)
                self.assertNotIn("NODE_OPTIONS", options["env"])
                self.assertEqual(set(options["env"]), set(environment) | {
                    "WARP_CLI_AGENT_PROTOCOL_VERSION", "GROK_HOOK_EVENT", "GROK_SESSION_ID",
                    "WARP_CLI_AGENT_NOTIFY_EXECUTABLE"})
                self.assertEqual(options["env"]["WARP_CLI_AGENT_NOTIFY_EXECUTABLE"], str(files[2][0]))
                self.assertEqual(options["cwd"], root)
                terminal_fd, diagnostic_fd = options["pass_fds"]
                self.assertTrue(os.isatty(terminal_fd))
                self.assertTrue(stat.S_ISREG(os.fstat(diagnostic_fd).st_mode))
                observed.update(terminal_fd=terminal_fd, diagnostic_fd=diagnostic_fd)
                raise RuntimeError("synthetic-spawn-failure")

            with patch("run_installed_grok_hook.subprocess.Popen", side_effect=fail_spawn):
                with self.assertRaisesRegex(RuntimeError, "synthetic-spawn-failure"):
                    verify_installed_hook(*files[0], *files[1], *files[2], root, "0.1.5", environment)
            self.assertEqual(environment, before_environment)
            self.assertEqual(set(root.iterdir()), before_files)
            for descriptor in observed.values():
                with self.assertRaises(OSError):
                    os.fstat(descriptor)


if __name__ == "__main__":
    unittest.main()
