#!/usr/bin/env python3
"""真实验收运行器必须同时取得测试匹配、退出成功及对应协议终态。"""

import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from run_codex_adapter_live import idle_crash_environment, missing_session_environment, verified_acceptance


class AcceptanceEvidenceTests(unittest.TestCase):
    def test_each_case_requires_current_success_and_matching_test(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        zero = "test result: ok. 0 passed; 0 failed; 0 ignored;"
        cases = {
            "lifecycle": {"event": "acceptance_passed"},
            "local-tools-restore": {"event": "tool_restore_probe_finished", "passed": True},
            "image-input": {"event": "image_probe_finished", "passed": True},
            "missing-session": {"event": "missing_session_probe_finished", "passed": True},
            "idle-crash": {"event": "idle_crash_probe_finished", "passed": True,
                           "phase": "after_session_ready", "native_root_exit_observed": True,
                           "credentials_provided": False, "model_commands_sent": 0},
        }
        for case, event in cases.items():
            with self.subTest(case=case):
                self.assertTrue(verified_acceptance(case, 0, summary, [event]))
                self.assertFalse(verified_acceptance(case, 1, summary, [event]))
                self.assertFalse(verified_acceptance(case, 0, zero, [event]))
                self.assertFalse(verified_acceptance(case, 0, summary, []))

    def test_image_check_cannot_reuse_another_probe_or_failed_result(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        for event in [
            {"event": "tool_restore_probe_finished", "passed": True},
            {"event": "image_probe_finished", "passed": False},
        ]:
            with self.subTest(event=event):
                self.assertFalse(verified_acceptance("image-input", 0, summary, [event]))

    def test_missing_session_cannot_reuse_another_probe_or_failed_result(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        for event in [
            {"event": "acceptance_passed"},
            {"event": "tool_restore_probe_finished", "passed": True},
            {"event": "missing_session_probe_finished", "passed": False},
        ]:
            with self.subTest(event=event):
                self.assertFalse(verified_acceptance("missing-session", 0, summary, [event]))

    def test_missing_session_environment_has_no_user_credentials_or_configuration(self):
        inherited = {"PATH": "fixture-bin", "SystemRoot": "fixture-system", "OPENAI_API_KEY": "test-only",
                     "HOME": "user-home", "CODEX_HOME": "user-codex", "CODEX_CONFIG": "user-config",
                     "HTTPS_PROXY": "user-proxy", "DYLD_INSERT_LIBRARIES": "user-library"}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, inherited, clear=True):
            root = Path(temporary).resolve()
            environment = missing_session_environment(root)
            self.assertEqual(environment["PATH"], "fixture-bin")
            self.assertEqual(environment["SystemRoot"], "fixture-system")
            for key in ("OPENAI_API_KEY", "CODEX_CONFIG", "HTTPS_PROXY", "DYLD_INSERT_LIBRARIES"):
                self.assertNotIn(key, environment)
            for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "CODEX_HOME", "TMP", "TEMP", "TMPDIR"):
                self.assertTrue(Path(environment[key]).is_relative_to(root))
                self.assertTrue(Path(environment[key]).is_dir())
            self.assertEqual((root / "codex/config.toml").read_bytes(), b'cli_auth_credentials_store = "file"\n')
            self.assertEqual((root / ".infinishell-missing-session-probe").read_bytes(),
                             b"isolated unauthenticated missing-session verification\n")
            self.assertFalse((root / "codex/auth.json").exists())
            self.assertEqual(environment["INFINISHELL_CODEX_MISSING_ROOT"], str(root))

    def test_idle_crash_requires_real_ready_root_exit_and_no_model(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        valid = {"event": "idle_crash_probe_finished", "passed": True,
                 "phase": "after_session_ready", "native_root_exit_observed": True,
                 "credentials_provided": False, "model_commands_sent": 0}
        for key, value in {"event": "missing_session_probe_finished", "passed": False,
                           "phase": "before_session_ready", "native_root_exit_observed": False,
                           "credentials_provided": True, "model_commands_sent": 1}.items():
            with self.subTest(key=key):
                self.assertFalse(verified_acceptance("idle-crash", 0, summary, [valid | {key: value}]))

    def test_idle_crash_environment_reuses_credential_free_boundary_with_own_marker(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ,
                {"OPENAI_API_KEY": "test-only", "CODEX_HOME": "user-home"}, clear=True):
            root = Path(temporary).resolve()
            environment = idle_crash_environment(root)
            self.assertNotIn("OPENAI_API_KEY", environment)
            self.assertNotIn("INFINISHELL_CODEX_MISSING_ROOT", environment)
            self.assertFalse((root / ".infinishell-missing-session-probe").exists())
            self.assertEqual(environment["INFINISHELL_CODEX_IDLE_CRASH_ROOT"], str(root))
            self.assertEqual((root / ".infinishell-idle-crash-probe").read_bytes(),
                             b"isolated unauthenticated idle-crash verification\n")
            self.assertEqual((root / "codex/config.toml").read_bytes(), b'cli_auth_credentials_store = "file"\n')
            self.assertFalse((root / "codex/auth.json").exists())


if __name__ == "__main__":
    unittest.main()
