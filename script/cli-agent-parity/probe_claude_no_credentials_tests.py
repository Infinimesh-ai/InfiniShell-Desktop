#!/usr/bin/env python3
"""无模型探测器的协议拒绝和管道回归；Python 子进程仅是传输夹具，不是原生 CLI 证据。"""

import copy
from contextlib import contextmanager
import json
import os
from pathlib import Path
import sys
import sysconfig
import tempfile
import unittest
from unittest import mock

import probe_claude_no_credentials as probe
from probe_claude_no_credentials import (Recorder, clean, command, isolated_environment,
                                        validate_initialize, validate_transcript)


def response(pid=42):
    return {"type": "control_response", "response": {"subtype": "success", "request_id": "init-probe",
        "response": {"pid": pid, "account": {"tokenSource": "none", "apiProvider": "firstParty"},
                     "session_state": "idle", "current_permission_mode": "default"},
        "pending_permission_requests": [], "pending_user_dialog_requests": []}}


def records():
    return [{"direction": "stdin", "message": {"type": "control_request", "request_id": "init-probe",
                "request": {"subtype": "initialize"}}}, {"direction": "stdout", "message": response()}]


def python_fixture_environment(root):
    env = isolated_environment(root)
    if sys.platform != "linux" or not sysconfig.get_config_var("Py_ENABLE_SHARED"):
        return env
    libdir = sysconfig.get_config_var("LIBDIR")
    library = sysconfig.get_config_var("LDLIBRARY")
    if not isinstance(libdir, str) or not Path(libdir).is_absolute():
        raise ValueError("共享 Python 夹具缺少绝对 LIBDIR")
    if not isinstance(library, str) or Path(library).name != library or not library.startswith("libpython"):
        raise ValueError("共享 Python 夹具缺少有效 LDLIBRARY")
    prefix = Path(sys.base_prefix).resolve(strict=True)
    directory = Path(libdir).resolve()
    if not directory.is_relative_to(prefix):
        # 固定 Actions 归档的 LIBDIR 是构建前缀；安装器按原布局复制到当前工具缓存。
        if Path(libdir).name != "lib":
            raise ValueError("无法证明共享 Python 夹具的重定位库目录")
        directory = prefix / "lib"
    directory = directory.resolve(strict=True)
    target = (directory / library).resolve(strict=True)
    if not directory.is_relative_to(prefix) or target.parent != directory or not target.is_file():
        raise ValueError("共享 Python 夹具的实际库文件不属于当前解释器目录")
    # 仅两个 Python 传输夹具需要解释器共享库，不继承任意 loader 环境或修改原生 CLI 隔离。
    env["LD_LIBRARY_PATH"] = str(directory)
    return env


@contextmanager
def python_transport_fixture(root, source):
    env = python_fixture_environment(root)
    recorder = Recorder([sys.executable, "-c", source], env, root)
    try:
        yield recorder
    except Exception as error:
        try:
            recorder.close()
        except Exception as cleanup_error:
            error.add_note(f"传输夹具清理结果：{cleanup_error}")
        diagnostics = {
            "python": sys.executable, "python_version": sys.version.split()[0],
            "library_directory": env.get("LD_LIBRARY_PATH"),
            "exit_code": recorder.process.poll(),
            "stderr": [str(row["message"])[:4096] for row in recorder.records
                       if row["direction"] == "stderr"][:4],
            "reader_errors": recorder.reader_errors[:4],
        }
        error.add_note("Python 子进程夹具诊断：" + json.dumps(clean(diagnostics, root), ensure_ascii=False))
        raise


class ClaudeNoCredentialsTests(unittest.TestCase):
    def test_selected_release_version_is_used_before_native_launch(self):
        executable = Path("/fixture/claude")
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.object(probe, "isolated_environment", return_value={}), \
             mock.patch.object(probe, "repository_identity", return_value={}), \
             mock.patch.object(probe, "current_platform", return_value="fixture-platform"), \
             mock.patch.object(probe, "verify_binary", side_effect=ValueError("stop")) as verify:
            with self.assertRaisesRegex(ValueError, "stop"):
                probe.run(executable, Path(temporary), {}, "2.1.278")
        verify.assert_called_once_with(executable, "fixture-platform", "2.1.278")

    def test_initialize_requires_exact_success_identity_and_empty_pending_queues(self):
        validate_initialize(response(), "init-probe", 42)
        paths = {
            ("response", "subtype"): "error",
            ("response", "request_id"): "old-request",
            ("response", "response", "pid"): 43,
            ("response", "response", "account", "tokenSource"): "environment",
            ("response", "response", "account", "apiProvider"): "bedrock",
            ("response", "response", "session_state"): "running",
            ("response", "response", "current_permission_mode"): "bypassPermissions",
            ("response", "pending_permission_requests"): [{"request_id": "approval"}],
            ("response", "pending_user_dialog_requests"): [{"request_id": "login"}],
        }
        for path, value in paths.items():
            broken = copy.deepcopy(response())
            target = broken
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(ValueError):
                validate_initialize(broken, "init-probe", 42)
        with self.assertRaises(ValueError):
            validate_initialize(response(pid=True), "init-probe", 1)

    def test_any_user_or_model_record_and_duplicate_output_are_rejected(self):
        validate_transcript(records(), "init-probe", 42)
        for kind in ("user", "assistant", "result", "stream_event"):
            for channel in ("stdin", "stdout", "stderr"):
                with self.subTest(kind=kind, channel=channel), self.assertRaises(ValueError):
                    validate_transcript(records() + [{"direction": channel, "message": {"type": kind}}], "init-probe", 42)
        with self.assertRaises(ValueError):
            validate_transcript(records() + [records()[1]], "init-probe", 42)
        with self.assertRaises(ValueError):
            validate_transcript(records()[:1], "init-probe", 42)

    def test_command_uses_host_permissions_and_disables_external_settings(self):
        arguments = command(Path("/fixture/claude"))
        self.assertEqual(arguments[arguments.index("--setting-sources") + 1], "")
        self.assertEqual(arguments[arguments.index("--permission-prompts") + 1], "host")
        self.assertIn("--strict-mcp-config", arguments)
        self.assertNotIn("--dangerously-skip-permissions", arguments)
        self.assertNotIn("--resume", arguments)

    def test_sensitive_fields_are_redacted_without_losing_token_source_evidence(self):
        root = Path(tempfile.gettempdir()).resolve() / "redaction-fixture"
        value = clean({"access_token": "test-only", "api_key": "test-only", "email": "test@example.invalid",
                       "account": {"tokenSource": "none"}, "cwd": f"{root}/project"}, root)
        self.assertEqual(value["access_token"], "<redacted>")
        self.assertEqual(value["email"], "<redacted>")
        self.assertEqual(value["account"], {"tokenSource": "none"})
        self.assertEqual(value["cwd"], "<isolated-probe>/project")

    def test_utf8_transport_closes_after_eof_without_forced_cleanup(self):
        source = "import json,os,sys; request=json.loads(sys.stdin.readline()); value=" + repr(response()) + "; value['response']['request_id']=request['request_id']; value['response']['response']['pid']=os.getpid(); value['response']['response']['description']='中文 English'; print(json.dumps(value,ensure_ascii=False),flush=True); sys.stdin.read()"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with python_transport_fixture(root, source) as recorder:
                try:
                    value = recorder.initialize("init-probe")
                    validate_initialize(value, "init-probe", recorder.process.pid)
                    result = recorder.finish_eof()
                finally:
                    forced = recorder.close()
                self.assertFalse(forced)
                self.assertTrue(result["stdin_eof_exited_within_5s"])
                self.assertEqual(result["exit_code_before_cleanup"], 0)
                validate_transcript(recorder.records, "init-probe", recorder.process.pid)

    def test_invalid_utf8_is_a_reported_failure_and_child_is_reaped(self):
        source = "import sys; sys.stdin.readline(); sys.stdout.buffer.write(b'\\xff\\n'); sys.stdout.buffer.flush()"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with python_transport_fixture(root, source) as recorder:
                try:
                    with self.assertRaises(ValueError):
                        recorder.initialize("init-probe")
                finally:
                    with self.assertRaises(ValueError):
                        recorder.close()
                self.assertTrue(recorder.reader_errors)
                self.assertIsNotNone(recorder.process.returncode)

    def test_python_fixture_selects_only_its_existing_shared_library_directory(self):
        for relocated in (False, True):
            with self.subTest(relocated=relocated), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                prefix = root / "interpreter"
                directory = prefix / "lib"
                directory.mkdir(parents=True)
                (directory / "libpython3.13.so").write_bytes(b"layout-fixture")
                values = {"Py_ENABLE_SHARED": 1, "LDLIBRARY": "libpython3.13.so",
                          "LIBDIR": str(root / "build-prefix/lib" if relocated else directory)}
                with mock.patch.object(sys, "platform", "linux"), mock.patch.object(sys, "base_prefix", str(prefix)), \
                     mock.patch.object(sysconfig, "get_config_var", side_effect=values.get), \
                     mock.patch.dict(os.environ, {"LD_LIBRARY_PATH": "/unrelated", "LD_PRELOAD": "/unrelated.so", "API_KEY": "test-only"}):
                    env = python_fixture_environment(root / "fixture")
                    self.assertEqual(env["LD_LIBRARY_PATH"], str(directory))
                    self.assertNotIn("LD_PRELOAD", env)
                    self.assertNotIn("API_KEY", env)
                    self.assertNotIn("LD_LIBRARY_PATH", isolated_environment(root / "native"))

    def test_python_fixture_rejects_invalid_or_missing_shared_library_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "lib").mkdir()
            for libdir, library in (("relative/lib", "libpython.so"), (str(root / "lib"), "../libpython.so"),
                                    (str(root / "lib"), "libpython-missing.so")):
                values = {"Py_ENABLE_SHARED": 1, "LIBDIR": libdir, "LDLIBRARY": library}
                with self.subTest(libdir=libdir, library=library), mock.patch.object(sys, "platform", "linux"), \
                     mock.patch.object(sys, "base_prefix", str(root)), \
                     mock.patch.object(sysconfig, "get_config_var", side_effect=values.get), \
                     self.assertRaises((ValueError, FileNotFoundError)):
                    python_fixture_environment(root / "fixture")

    def test_early_python_exit_reports_stderr_and_exit_code(self):
        source = "import sys; sys.stdin.readline(); sys.stderr.write('fixture-loader-diagnostic\\n'); sys.exit(37)"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with self.assertRaises(ValueError) as caught:
                with python_transport_fixture(root, source) as recorder:
                    recorder.initialize("init-probe")
            diagnostic = "\n".join(caught.exception.__notes__)
            self.assertIn('"exit_code": 37', diagnostic)
            self.assertIn("fixture-loader-diagnostic", diagnostic)
            self.assertIsNotNone(recorder.process.returncode)


if __name__ == "__main__":
    unittest.main()
