#!/usr/bin/env python3
"""无模型探测器的协议拒绝和管道回归；Python 子进程仅是传输夹具，不是原生 CLI 证据。"""

import copy
from pathlib import Path
import sys
import tempfile
import unittest

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


class ClaudeNoCredentialsTests(unittest.TestCase):
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
            recorder = Recorder([sys.executable, "-c", source], isolated_environment(root), root)
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
            recorder = Recorder([sys.executable, "-c", source], isolated_environment(root), root)
            try:
                with self.assertRaises(ValueError):
                    recorder.initialize("init-probe")
            finally:
                with self.assertRaises(ValueError):
                    recorder.close()
            self.assertTrue(recorder.reader_errors)
            self.assertIsNotNone(recorder.process.returncode)


if __name__ == "__main__":
    unittest.main()
