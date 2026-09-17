#!/usr/bin/env python3
"""权限安全投影和比较回归；不启动 CLI，不把资料比较当作实际工具授权。"""

import copy
import json
from pathlib import Path, PurePosixPath, PureWindowsPath
import queue
import sys
import unittest
from types import SimpleNamespace

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_claude_permission_snapshot as probe


def snapshot():
    settings = probe.project_settings({"effective": copy.deepcopy(probe.BASE),
                                       "sources": [{"source": "flagSettings", "settings": copy.deepcopy(probe.BASE)}],
                                       "applied": {"model": "unused"}})
    rules = [{"behavior": behavior, "source": "flagSettings", "rule": rule, "editability": "readonly"}
             for behavior in ("allow", "deny", "ask") for rule in probe.BASE["permissions"][behavior]]
    return {"mode_before": "default", "mode_after": "default", "native_pid": 1, "settings": settings,
            "rules": probe.project_rules({"state": {"rules": rules, "workspaceDirectories": [],
                                                      "originalCwd": "/isolated/project", "managedOnly": False}})}


class ProjectionTests(unittest.TestCase):
    def test_nonpermission_secrets_never_enter_projection_and_are_not_silently_accepted(self):
        value = {"effective": {**copy.deepcopy(probe.BASE), "env": {"ANTHROPIC_API_KEY": "secret-effective"}},
                 "sources": [{"source": "flagSettings", "settings": {**copy.deepcopy(probe.BASE),
                               "secret-as-a-key": {"secret": "secret-source"}}}],
                 "applied": {"secret": "secret-applied"}, "errors": [{"secret": "secret-error"}]}
        projected = probe.project_settings(value)
        text = json.dumps(projected)
        self.assertNotIn("secret", text)
        self.assertNotIn("ANTHROPIC", text)
        self.assertEqual(projected["unknown_fields"], [{"path": "effective", "unknown_field_count": 1},
                                                     {"path": "source.settings", "unknown_field_count": 1}])
        self.assertTrue(projected["reported_errors"])

    def test_unknown_sandbox_field_is_marked_without_its_key_or_value(self):
        value = {"effective": {"sandbox": {"enabled": False, "unknown-secret-key": "private"}}, "sources": []}
        projected = probe.project_settings(value)
        self.assertEqual(projected["effective"], {"sandbox": {"enabled": False}})
        self.assertEqual(projected["unknown_fields"], [{"path": "effective.sandbox", "unknown_field_count": 1}])
        self.assertNotIn("private", json.dumps(projected))

    def test_wrong_scalar_and_unknown_source_fail_closed(self):
        for value in ({"effective": {"sandbox": {"enabled": 0}}, "sources": []},
                      {"effective": {}, "sources": [{"source": "future", "settings": {}}]}):
            with self.assertRaises(ValueError):
                probe.project_settings(value)

    def test_missing_sandbox_is_not_false(self):
        value = snapshot()
        del value["settings"]["effective"]["sandbox"]
        self.assertIn("sandbox_not_explicitly_disabled", probe.consistency(value))

    def test_settings_and_live_rules_disagreement_is_rejected(self):
        value = snapshot()
        value["settings"]["sources"][0]["settings"]["permissions"]["deny"].append("Read(./later/**)")
        self.assertIn("settings_live_rules_mismatch", probe.consistency(value))
        self.assertFalse(probe.compare(value, value)["equal_and_consistent_observation"])

    def test_managed_only_and_inactive_rules_are_not_claimed_equivalent(self):
        value = snapshot()
        value["rules"]["managedOnly"] = True
        value["rules"]["rules"][0]["notInEffect"] = True
        self.assertEqual(probe.consistency(value), ["managed_only_unverified", "settings_live_rules_mismatch"])

    def test_unknown_live_state_and_rule_fields_are_marked(self):
        value = {"state": {"rules": [{"behavior": "deny", "source": "session", "rule": "Bash",
                                      "editability": "session", "private-key": "secret"}],
                           "workspaceDirectories": [], "originalCwd": "/private", "managedOnly": False,
                           "future": "secret2"}}
        projected = probe.project_rules(value)
        self.assertEqual(len(projected["unknown_fields"]), 2)
        self.assertEqual(projected["rules"][0]["source"], "session")
        self.assertNotIn("secret", json.dumps(projected))

    def test_same_controlled_snapshot_is_data_equality_never_dispatch_authorization(self):
        parent, child = snapshot(), snapshot()
        child["native_pid"] = 2
        result = probe.compare(parent, child)
        self.assertTrue(result["equal_and_consistent_observation"])
        self.assertFalse(result["dispatch_authorized"])
        self.assertFalse(result["atomic_permission_ceiling_proven"])

    def test_mode_drift_between_queries_and_old_child_are_rejected(self):
        parent, child = snapshot(), snapshot()
        parent["mode_after"] = "dontAsk"
        result = probe.compare(parent, child)
        self.assertIn("mode_changed_during_queries", result["reasons"])
        self.assertFalse(result["equal_and_consistent_observation"])

    def test_source_cwd_additional_directory_and_session_grant_are_not_erased(self):
        original = snapshot()
        variants = [snapshot() for _ in range(4)]
        variants[0]["rules"]["originalCwd"] = "/other"
        variants[1]["rules"]["workspaceDirectories"] = [{"path": "/other", "source": "cliArg"}]
        variants[2]["rules"]["rules"][0]["source"] = "userSettings"
        variants[3]["rules"]["rules"].append({"source": "session", "behavior": "allow", "rule": "Bash", "editability": "session"})
        for value in variants:
            self.assertFalse(probe.compare(original, value)["equal_and_consistent_observation"])

    def test_path_scrub_only_changes_private_anchor(self):
        value = {"path": "/private/probe/project", "rule": "Read(./literal/**)"}
        self.assertEqual(probe.scrub(value, PurePosixPath("/private/probe")),
                         {"path": "<isolated-probe>/project", "rule": "Read(./literal/**)"})

    def test_path_scrub_handles_windows_native_and_forward_slash_paths(self):
        root = PureWindowsPath(r"C:\private\probe")
        value = {"paths": [str(root / "project 中文"), (root / "project 中文").as_posix()],
                 "rule": "Read(./literal/**)"}
        self.assertEqual(probe.scrub(value, root),
                         {"paths": ["<isolated-probe>\\project 中文", "<isolated-probe>/project 中文"],
                          "rule": "Read(./literal/**)"})


class ProtocolTests(unittest.TestCase):
    def session(self):
        session = object.__new__(probe.Session)
        session.report = {"notifications": [], "queries": []}
        session.native_session_id = None
        session.answered = set()
        session.sequence = 0
        return session

    def test_only_observed_idle_notifications_are_accepted(self):
        session = self.session()
        session.notification({"type": "system", "subtype": "status", "status": None, "permissionMode": "dontAsk",
                              "uuid": "status", "session_id": "owned-session"})
        session.notification({"type": "system", "subtype": "background_tasks_changed", "tasks": [],
                              "uuid": "tasks", "session_id": "owned-session"})
        self.assertEqual(len(session.report["notifications"]), 2)
        for invalid in ({"type": "assistant"}, {"type": "system", "subtype": "init"},
                        {"type": "system", "subtype": "background_tasks_changed", "tasks": ["unexpected"]},
                        {"type": "system", "subtype": "status", "status": None, "permissionMode": "default", "session_id": "old-session"}):
            with self.assertRaises(ValueError):
                session.notification(invalid)

    def test_model_request_and_permission_bypass_are_never_sent(self):
        session = self.session()
        with self.assertRaises(ValueError):
            session.query("user", content="do anything")
        with self.assertRaises(ValueError):
            session.query("set_permission_mode", mode="bypassPermissions")
        self.assertEqual(session.report["queries"], [])

    def test_wrong_request_id_is_rejected_before_marking_success(self):
        session = self.session()
        messages = queue.Queue()
        messages.put({"type": "control_response", "response": {"request_id": "old", "subtype": "success"}})
        session.recorder = SimpleNamespace(messages=messages, reader_errors=[],
                                          process=SimpleNamespace(stdin=SimpleNamespace(write=lambda _: None, flush=lambda: None)))
        with self.assertRaises(ValueError):
            session.query("get_settings")
        self.assertFalse(session.report["queries"][0]["success"])


if __name__ == "__main__":
    unittest.main()
