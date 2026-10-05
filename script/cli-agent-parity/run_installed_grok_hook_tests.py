"""同一次 hook 原生调用的有限诊断协议；不执行 Node 或通知 worker。"""

import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, call, patch

import installed_grok_hook_tests as detached
from run_installed_grok_hook import DIAGNOSTIC_LIMIT, hook_failure, verify_installed_hook, worker_diagnostics
from run_installed_grok_hook import (NATIVE_TRACE_LIMIT, NATIVE_TRACE_RECORD, NativeWorkerTrace,
                                     native_worker_diagnostics)


def row(stage, **values):
    return {"stage": stage, "elapsed_ms": 0, "exit_code": None, "signal": None,
            "stdout_bytes": 0, "stderr_bytes": 0, "error_code": None, **values}


def encode(*rows):
    return ("\n".join(json.dumps(value) for value in rows) + "\n").encode()


class WorkerDiagnosticTests(unittest.TestCase):
    def test_guarded_send_has_no_synthetic_protocol_stage(self):
        rows = [row("preload"), row("send", exit_code=0, elapsed_ms=2.5,
                                   stdout_bytes=len(b'{"protocol":1,"maxFrameBytes":4096}\n'),
                                   stderr_bytes=None)]
        self.assertEqual(worker_diagnostics(encode(*rows)), {"status": "captured", "events": rows})

    def test_guarded_send_protocol_mismatch_and_timeout_keep_the_send_stage(self):
        for values in ({"exit_code": 1, "error_code": "cli_agent_notify_protocol_mismatch"},
                       {"signal": "SIGKILL", "error_code": "ETIMEDOUT", "elapsed_ms": 2600}):
            with self.subTest(values=values):
                rows = [row("preload"), row("send", **values)]
                self.assertEqual(worker_diagnostics(encode(*rows)), {"status": "captured", "events": rows})

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
        cases = [[row("send")], [row("protocol"), row("send")],
                 [row("preload"), row("send"), row("send")],
                 [row("preload"), row("send"), row("protocol")],
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
                terminal_fd, diagnostic_fd, *native_fds = options["pass_fds"]
                self.assertTrue(os.isatty(terminal_fd))
                self.assertTrue(stat.S_ISREG(os.fstat(diagnostic_fd).st_mode))
                observed.update(terminal_fd=terminal_fd, diagnostic_fd=diagnostic_fd)
                for index, descriptor in enumerate(native_fds):
                    self.assertTrue(stat.S_ISFIFO(os.fstat(descriptor).st_mode))
                    self.assertFalse(os.get_blocking(descriptor))
                    observed[f"native_{index}"] = descriptor
                raise RuntimeError("synthetic-spawn-failure")

            with patch("run_installed_grok_hook.subprocess.Popen", side_effect=fail_spawn):
                with self.assertRaisesRegex(RuntimeError, "synthetic-spawn-failure"):
                    verify_installed_hook(*files[0], *files[1], *files[2], root, "0.1.6", environment)
            self.assertEqual(environment, before_environment)
            self.assertEqual(set(root.iterdir()), before_files)
            for descriptor in observed.values():
                with self.assertRaises(OSError):
                    os.fstat(descriptor)


@unittest.skipUnless(os.name == "posix", "脱离控制终端的 helper 仅适用于 Unix")
class DetachedWorkerDiagnosticTests(unittest.TestCase):
    def setUp(self):
        # 只调用原测试驱动并替换派生边界，不执行 Node、worker 或创建 PTY。
        temporary = tempfile.TemporaryDirectory(prefix="detached-diagnostic-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.case = detached.DetachedGrokHookTests()
        self.case.root = self.root
        self.case.node = self.root / "node"
        self.case.worker = self.root / "worker"
        self.case.hook = self.root / "notify.cjs"
        self.case.env = {"HOME": str(self.root), "PATH": "/usr/bin:/bin", "TMPDIR": str(self.root)}
        self.case.payload = b"synthetic-no-execution"
        self.case.terminals = [(123, 124, "/dev/pts/isolated")]
        self.case.hook_diagnostics = []

    def test_detached_deadline_retains_protocol_failure_without_retry_or_raw_text(self):
        before = dict(self.case.env)
        descriptors = []
        processes = []
        process_exit = subprocess.Popen.__exit__
        calls = [encode(row("preload"), row("protocol", elapsed_ms=500, signal="SIGKILL",
                                           error_code="ETIMEDOUT")),
                 encode(row("preload", private="private-sentinel"))]

        def observe_call(argv, **options):
            self.assertEqual(argv[0], str(self.case.node))
            self.assertEqual(argv[1], "--require")
            self.assertEqual(argv[3], "-e")
            self.assertIn("ENXIO", argv[4])
            self.assertEqual(argv[5], str(self.case.hook))
            self.assertEqual(options["env"], {**before, "SSH_TTY": "/dev/pts/isolated"})
            self.assertTrue(options["start_new_session"])
            self.assertEqual(set(options), {"stdin", "stdout", "stderr", "start_new_session", "env", "pass_fds"})
            for stream in ("stdin", "stdout", "stderr"):
                self.assertEqual(options[stream], subprocess.PIPE)
            descriptor, *native_fds = options["pass_fds"]
            self.assertTrue(stat.S_ISREG(os.fstat(descriptor).st_mode))
            self.assertEqual(stat.S_IMODE(Path(argv[2]).stat().st_mode), 0o600)
            self.assertIn("child.execFileSync", Path(argv[2]).read_text())
            os.write(descriptor, calls.pop(0))
            descriptors.append(descriptor)
            descriptors.extend(native_fds)
            process = MagicMock()
            process.pid, process.args = 0, argv
            process.__enter__.return_value = process
            process.__exit__.side_effect = lambda *args: process_exit(process, *args)
            process.communicate.return_value = (b"", b"")
            process.poll.return_value = 0
            processes.append(process)
            return process

        with patch.object(detached.subprocess, "Popen", side_effect=observe_call) as run:
            self.case.run_hook({"SSH_TTY": "/dev/pts/isolated"})
            self.assertEqual(run.call_count, 1)
            with patch.object(detached, "collect_terminal_output", side_effect=AssertionError("terminal_read_deadline")):
                with self.assertRaisesRegex(AssertionError, "terminal_read_deadline") as failure:
                    self.case.output(0, expect_notification=True)
            self.assertIn('"error_code":"ETIMEDOUT"', failure.exception.__notes__[0])
            self.assertEqual(run.call_count, 1)
            self.case.run_hook({"SSH_TTY": "/dev/pts/isolated"})
            with self.assertRaises(AssertionError) as invalid_frame:
                self.case.assert_notification(b"invalid-frame")
            self.assertIn('"status":"invalid"', str(invalid_frame.exception))
            self.assertIn('"error_code":"ETIMEDOUT"', str(invalid_frame.exception))
            self.assertNotIn("private-sentinel", str(invalid_frame.exception))
        self.assertEqual(self.case.env, before)
        self.assertEqual(list(self.root.iterdir()), [])
        for process in processes:
            process.communicate.assert_called_once_with(self.case.payload, timeout=8)
            process.kill.assert_not_called()
            process.wait.assert_called_once_with()
            for stream in (process.stdin, process.stdout, process.stderr):
                stream.close.assert_called_once_with()
        for descriptor in descriptors:
            with self.assertRaises(OSError):
                os.fstat(descriptor)

    def test_detached_timeout_preserves_live_or_exited_root_and_original_exception(self):
        process_exit = subprocess.Popen.__exit__
        for returncode, output, stderr in ((None, None, b""), (0, b"private-sentinel-output", None)):
            with self.subTest(returncode=returncode):
                error = subprocess.TimeoutExpired("synthetic-no-execution", 8, output=output, stderr=stderr)
                descriptors = []
                process = MagicMock()
                process.pid = 12345
                process.__enter__.return_value = process
                process.__exit__.side_effect = lambda *args: process_exit(process, *args)
                process.communicate.side_effect = error
                process.poll.return_value = returncode

                def spawn(argv, **options):
                    descriptor, *native_fds = options["pass_fds"]
                    os.write(descriptor, encode(row("preload")))
                    descriptors.extend((descriptor, *native_fds))
                    return process

                with patch.object(detached.subprocess, "Popen", side_effect=spawn) as run, \
                        patch.object(detached.sys, "platform", "linux"), \
                        patch.object(detached.Path, "open", autospec=True) as birth_open, \
                        patch.object(detached.time, "monotonic_ns", return_value=123456789):
                    birth_file = birth_open.return_value.__enter__.return_value
                    birth_file.read.return_value = b"12345 (node) private) R " + b"0 " * 18 + b"98765 0\n"
                    with self.assertRaises(subprocess.TimeoutExpired) as failure:
                        self.case.run_hook({})
                    self.assertIs(failure.exception, error)
                    self.assertEqual(run.call_count, 1)
                    birth_open.assert_called_once_with(Path("/proc/12345/stat"), "rb")
                    birth_file.read.assert_called_once_with(4097)
                process.assert_has_calls([call.communicate(self.case.payload, timeout=8), call.poll(),
                                          call.kill(), call.wait()])
                process.communicate.assert_called_once_with(self.case.payload, timeout=8)
                process.kill.assert_called_once_with()
                self.assertEqual(process.wait.call_args_list, [call(), call()])
                for stream in (process.stdin, process.stdout, process.stderr):
                    stream.close.assert_called_once_with()
                self.assertEqual(self.case.hook_diagnostics[-1]["node_diagnostics"], {
                    "pid": 12345, "linux_starttime_ticks": 98765, "timeout_poll_monotonic_ns": 123456789,
                    "timeout_poll_succeeded": True, "timeout_returncode": returncode,
                    "timeout_stdout_bytes": None if output is None else len(output),
                    "timeout_stderr_bytes": None if stderr is None else len(stderr)})
                self.assertIs(error.stdout, output)
                self.assertIs(error.stderr, stderr)
                self.assertIn('"stage":"preload"', error.__notes__[0])
                self.assertNotIn("private-sentinel", error.__notes__[0])
                self.assertEqual(list(self.root.iterdir()), [])
                for descriptor in descriptors:
                    with self.assertRaises(OSError):
                        os.fstat(descriptor)

    def test_unavailable_or_invalid_birth_and_poll_leave_timeout_evidence_unknown(self):
        process_exit = subprocess.Popen.__exit__
        for birth in (OSError("private-sentinel"), b"x" * 4097, b"invalid-stat"):
            with self.subTest(birth_type=type(birth).__name__):
                error = subprocess.TimeoutExpired("synthetic-no-execution", 8)
                process = MagicMock()
                process.pid = 12345
                process.__enter__.return_value = process
                process.__exit__.side_effect = lambda *args: process_exit(process, *args)
                process.communicate.side_effect = error
                process.poll.side_effect = OSError("private-sentinel")
                with patch.object(detached.subprocess, "Popen", return_value=process) as run, \
                        patch.object(detached.sys, "platform", "linux"), \
                        patch.object(detached.Path, "open", autospec=True) as birth_open:
                    birth_file = birth_open.return_value.__enter__.return_value
                    if isinstance(birth, OSError):
                        birth_file.read.side_effect = birth
                    else:
                        birth_file.read.return_value = birth
                    with self.assertRaises(subprocess.TimeoutExpired) as failure:
                        self.case.run_hook({})
                    self.assertIs(failure.exception, error)
                    run.assert_called_once()
                    birth_open.assert_called_once_with(Path("/proc/12345/stat"), "rb")
                    birth_file.read.assert_called_once_with(4097)
                observed = self.case.hook_diagnostics[-1]["node_diagnostics"]
                self.assertIsNone(observed["linux_starttime_ticks"])
                self.assertIsNone(observed["timeout_returncode"])
                self.assertFalse(observed["timeout_poll_succeeded"])
                self.assertIsNone(observed["timeout_stdout_bytes"])
                self.assertIsNone(observed["timeout_stderr_bytes"])
                self.assertNotIn("private-sentinel", error.__notes__[0])
                process.communicate.assert_called_once_with(self.case.payload, timeout=8)
                process.kill.assert_called_once_with()
                self.assertEqual(process.wait.call_args_list, [call(), call()])
                self.assertEqual(list(self.root.iterdir()), [])

    def test_communicate_base_exception_keeps_original_kill_and_context_cleanup(self):
        process_exit = subprocess.Popen.__exit__
        for error in (RuntimeError("synthetic-failure"), KeyboardInterrupt()):
            with self.subTest(exception_type=type(error).__name__):
                process = MagicMock()
                process.pid, process._sigint_wait_secs = 0, 0
                process.__enter__.return_value = process
                process.__exit__.side_effect = lambda *args: process_exit(process, *args)
                process.communicate.side_effect = error
                with patch.object(detached.subprocess, "Popen", return_value=process) as run:
                    with self.assertRaises(type(error)) as failure:
                        self.case.run_hook({})
                    self.assertIs(failure.exception, error)
                    run.assert_called_once()
                process.communicate.assert_called_once_with(self.case.payload, timeout=8)
                process.kill.assert_called_once_with()
                process.poll.assert_not_called()
                self.assertEqual(process.wait.call_count, 0 if isinstance(error, KeyboardInterrupt) else 1)
                for stream in (process.stdin, process.stdout, process.stderr):
                    stream.close.assert_called_once_with()
                self.assertEqual(list(self.root.iterdir()), [])

    def test_nonzero_exit_preserves_called_process_error_without_kill(self):
        process_exit = subprocess.Popen.__exit__
        process = MagicMock()
        process.pid, process.args = 0, ["synthetic-no-execution"]
        process.__enter__.return_value = process
        process.__exit__.side_effect = lambda *args: process_exit(process, *args)
        process.communicate.return_value = (b"private-sentinel-output", b"private-sentinel-error")
        process.poll.return_value = 7
        with patch.object(detached.subprocess, "Popen", return_value=process) as run:
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                self.case.run_hook({})
            run.assert_called_once()
        self.assertEqual(failure.exception.returncode, 7)
        self.assertIs(failure.exception.cmd, process.args)
        self.assertEqual((failure.exception.stdout, failure.exception.stderr), process.communicate.return_value)
        self.assertNotIn("private-sentinel", failure.exception.__notes__[0])
        process.communicate.assert_called_once_with(self.case.payload, timeout=8)
        process.kill.assert_not_called()
        process.wait.assert_called_once_with()
        for stream in (process.stdin, process.stdout, process.stderr):
            stream.close.assert_called_once_with()
        self.assertEqual(list(self.root.iterdir()), [])


class NativeWorkerDiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.nonce = bytes.fromhex("ab" * 16)

    def record(self, stage=1, sequence=0, pid=4321, timestamp=500):
        return NATIVE_TRACE_RECORD.pack(b"INW1", stage, sequence, pid, timestamp, self.nonce)

    def test_same_invocation_records_preserve_phase_pid_and_monotonic_time(self):
        raw = self.record() + self.record(stage=2, sequence=1, timestamp=600)
        result = native_worker_diagnostics(raw, self.nonce)
        self.assertEqual(result["status"], "captured")
        self.assertEqual(result["events"], [
            {"stage": "main", "sequence": 0, "pid": 4321, "monotonic_ns": 500},
            {"stage": "fluent_begin", "sequence": 1, "pid": 4321, "monotonic_ns": 600}])
        self.assertTrue(result["missing_stages_are_unknown"])
        self.assertEqual(result["wire_sha256"], hashlib.sha256(raw).hexdigest())
        rebuilt = b"".join(NATIVE_TRACE_RECORD.pack(b"INW1", stage, index, 4321, timestamp,
                              bytes.fromhex(result["invocation_nonce"]))
                           for index, (stage, timestamp) in enumerate(((1, 500), (2, 600))))
        self.assertEqual(rebuilt, raw)
        self.assertEqual(native_worker_diagnostics(b"", self.nonce)["status"], "missing")

    def test_foreign_nonce_pid_sequence_time_and_reserved_data_are_rejected(self):
        invalid = [self.record(stage=0), self.record(stage=20), self.record(pid=0),
                   self.record(sequence=1), self.record()[:-1],
                   self.record() + self.record(sequence=1, pid=4322),
                   self.record() + self.record(sequence=1, timestamp=499),
                   self.record()[:5] + b"x" + self.record()[6:],
                   self.record()[:-1] + b"x"]
        for raw in invalid:
            with self.subTest(raw=raw):
                self.assertEqual(native_worker_diagnostics(raw, self.nonce)["status"], "invalid")
        self.assertEqual(native_worker_diagnostics(self.record(), bytes(16))["status"], "invalid")

    def test_payload_and_oversized_bytes_are_not_exported(self):
        for raw in (b"private-payload-and-path".ljust(48, b"x"), bytes(NATIVE_TRACE_LIMIT + 1)):
            result = native_worker_diagnostics(raw, self.nonce)
            self.assertNotIn("events", result)
            self.assertNotIn("private-payload", json.dumps(result))
        self.assertEqual(native_worker_diagnostics(bytes(NATIVE_TRACE_LIMIT + 1), self.nonce)["status"],
                         "over_budget")

    def test_pipe_is_nonblocking_read_once_and_closes_after_failure(self):
        with self.assertRaisesRegex(RuntimeError, "original-failure"):
            with NativeWorkerTrace(enabled=True) as trace:
                descriptors = (trace.reader, trace.writer)
                for descriptor in descriptors:
                    self.assertFalse(os.get_blocking(descriptor))
                    self.assertFalse(os.get_inheritable(descriptor))
                self.assertEqual(trace.summary()["status"], "missing")
                info = os.fstat(trace.writer)
                self.assertEqual(trace.authorization, f"v1:{info.st_dev}:{info.st_ino}:{trace.nonce.hex()}")
                raw = NATIVE_TRACE_RECORD.pack(b"INW1", 1, 0, 4321, 500, trace.nonce)
                os.write(trace.writer, raw)
                first = trace.summary()
                self.assertEqual(first["status"], "captured")
                self.assertEqual(first, trace.summary())
                raise RuntimeError("original-failure")
        for descriptor in descriptors:
            with self.assertRaises(OSError):
                os.fstat(descriptor)

    def test_pipe_setup_failure_is_unavailable_without_replacing_business_result(self):
        with patch("run_installed_grok_hook.os.pipe", side_effect=OSError("private-path")):
            with NativeWorkerTrace(enabled=True) as trace:
                self.assertEqual(trace.pass_fds, ())
                self.assertIsNone(trace.binding)
                self.assertEqual(trace.summary(), {"status": "unavailable"})
        with NativeWorkerTrace(enabled=False) as trace:
            self.assertEqual(trace.pass_fds, ())
            self.assertEqual(trace.summary(), {"status": "not_enabled"})

    def test_detached_failure_retains_native_phases_without_an_extra_send(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            case = detached.DetachedGrokHookTests()
            case.root = root
            case.node, case.worker, case.hook = root / "node", root / "worker", root / "notify.cjs"
            case.env = {"HOME": temporary, "PATH": "/usr/bin:/bin", "TMPDIR": temporary}
            case.payload = b"private-sentinel-payload"
            case.hook_diagnostics = []
            created = []
            original = subprocess.TimeoutExpired("original-command", 8)
            process_exit = subprocess.Popen.__exit__
            process = MagicMock()
            process.pid = 0
            process.__enter__.return_value = process
            process.__exit__.side_effect = lambda *args: process_exit(process, *args)
            process.communicate.side_effect = original
            process.poll.return_value = None

            def trace_factory():
                trace = NativeWorkerTrace(enabled=True)
                created.append(trace)
                return trace

            def spawn(argv, **options):
                trace = created[0]
                self.assertEqual(options["env"], case.env)
                diagnostic_fd, native_fd = options["pass_fds"]
                self.assertEqual(native_fd, trace.writer)
                os.write(diagnostic_fd, encode(row("preload")))
                os.write(native_fd, NATIVE_TRACE_RECORD.pack(b"INW1", 2, 0, 4321, 500, trace.nonce))
                return process

            with patch.object(detached, "NativeWorkerTrace", side_effect=trace_factory), \
                    patch.object(detached.subprocess, "Popen", side_effect=spawn) as run:
                with self.assertRaises(subprocess.TimeoutExpired) as failure:
                    case.run_hook({})
                self.assertIs(failure.exception, original)
                self.assertEqual(run.call_count, 1)
            process.communicate.assert_called_once_with(case.payload, timeout=8)
            process.kill.assert_called_once_with()
            self.assertEqual(process.wait.call_args_list, [call(), call()])
            for stream in (process.stdin, process.stdout, process.stderr):
                stream.close.assert_called_once_with()
            self.assertIn('"stage":"fluent_begin"', original.__notes__[0])
            self.assertNotIn("private-sentinel", original.__notes__[0])
            self.assertIsNone(created[0].reader)
            self.assertIsNone(created[0].writer)
            self.assertEqual(list(root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
