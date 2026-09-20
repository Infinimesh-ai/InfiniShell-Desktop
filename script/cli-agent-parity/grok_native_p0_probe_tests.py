#!/usr/bin/env python3
"""原始 ACP P0 的纯协议和合成进程回归；不运行 Grok、网络或认证。"""

import copy
import io
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

import grok_native_p0_probe as probe

SESSION = "00000000-0000-4000-8000-000000000001"
OTHER_SESSION = "00000000-0000-4000-8000-000000000002"


def permission(target, session=SESSION, call="call", request_id="permission"):
    return {"jsonrpc": "2.0", "id": request_id, "method": "session/request_permission", "params": {
        "sessionId": session, "toolCall": {"toolCallId": call, "kind": "read",
            "_meta": {"x.ai/tool": {"name": "read_file", "namespace": "grok_build", "version": 1, "read_only": True}},
            "rawInput": {"variant": "ReadFile", "target_file": str(target)}},
        "options": [{"optionId": "allow-once", "kind": "allow_once"},
                    {"optionId": "reject-once", "kind": "reject_once"}]}}


def history(output="answer", turn="turn", reason="end_turn", category=None):
    metadata = {"eventId": SESSION + "-2"}
    if category is not None:
        metadata.update(cancellationCategory=category, cancellationContext={"tool_name": "read_file"})
    return {"updates": [
        {"method": "session/update", "params": {"sessionId": SESSION,
            "_meta": {"promptId": turn, "eventId": SESSION + "-1"},
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": output}}}},
        {"method": "_x.ai/session/update", "params": {"sessionId": SESSION, "_meta": metadata,
            "update": {"sessionUpdate": "turn_completed", "prompt_id": turn, "stop_reason": reason}}}],
        "totalCount": 2, "hasMore": False, "lastEventId": SESSION + "-2"}


class FakeWire:
    def __init__(self, project, saved=None):
        self.project = project
        self.messages = queue.Queue()
        self.sent = []
        self.saved = [] if saved is None else saved
        self.input = 0 if saved is None else 3
        self.rpc_id = None
        self.turn = None
        self.live_event = 10000 if saved is None else 20000

    def response(self, request_id, result):
        self.messages.put({"jsonrpc": "2.0", "id": request_id, "result": result})

    def notification(self, method, params):
        self.live_event += 1
        params.setdefault("_meta", {})["eventId"] = SESSION + "-" + str(self.live_event)
        self.messages.put({"jsonrpc": "2.0", "method": method, "params": params})

    def record(self, method, update, category=None):
        meta = {"eventId": SESSION + "-" + str(len(self.saved) + 1), "promptId": self.turn}
        if category:
            meta.update(cancellationCategory=category, cancellationContext={"tool_name": "read_file"})
        self.saved.append({"method": method, "params": {"sessionId": SESSION, "_meta": meta, "update": update}})

    def text(self, text):
        update = {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}
        self.record("session/update", update)
        self.notification("session/update", {"sessionId": SESSION, "_meta": {"promptId": self.turn}, "update": update})

    def finish(self, reason, category=None):
        self.record("_x.ai/session/update", {"sessionUpdate": "turn_completed", "prompt_id": self.turn,
                    "stop_reason": reason}, category)
        meta = {"sessionId": SESSION, "promptId": self.turn}
        if category:
            meta["cancellationCategory"] = category
        self.response(self.rpc_id, {"stopReason": reason, "_meta": meta})

    def send(self, value):
        self.sent.append(copy.deepcopy(value))
        method = value.get("method")
        if method == "initialize":
            self.response(value["id"], {"protocolVersion": 1, "_meta": {"agentVersion": probe.VERSION},
                "agentCapabilities": {"loadSession": True, "promptCapabilities": {"image": False}},
                "authMethods": [{"id": "cached_token"}]})
        elif method in ("authenticate", "session/load"):
            self.response(value["id"], {})
        elif method == "session/new":
            self.response(value["id"], {"sessionId": SESSION})
        elif method == "session/prompt":
            self.rpc_id = value["id"]
            self.input += 1
            self.turn = "turn" + str(self.input)
            self.notification("_x.ai/queue/changed", {"sessionId": SESSION, "entries": [{"id": self.turn, "kind": "prompt"}]})
            self.notification("_x.ai/queue/changed", {"sessionId": SESSION, "entries": [], "runningPromptId": self.turn})
            if self.input <= 2:
                if self.input == 2:
                    self.text("等待审批。")
                call = "call" + str(self.input)
                update = {"sessionUpdate": "tool_call", "toolCallId": call, "status": "pending"}
                self.record("session/update", update)
                self.notification("session/update", {"sessionId": SESSION, "_meta": {"promptId": self.turn}, "update": update})
                self.messages.put(permission(self.project / ("allow.txt" if self.input == 1 else "deny.txt"), call=call,
                                             request_id="permission" + str(self.input)))
            elif self.input == 3:
                self.text("READY")
            else:
                self.text((self.project / "allow.txt").read_text())
                self.finish("end_turn")
        elif method == "session/cancel":
            self.finish("cancelled", "MidTurnAbort")
        elif method == "_x.ai/session/updates":
            self.response(value["id"], {"updates": copy.deepcopy(self.saved), "totalCount": len(self.saved),
                "hasMore": False, "lastEventId": self.saved[-1]["params"]["_meta"]["eventId"]})
        elif "result" in value:
            allowed = value["result"]["outcome"].get("optionId") == "allow-once"
            update = {"sessionUpdate": "tool_call_update", "toolCallId": "call" + str(self.input),
                      "status": "completed" if allowed else "cancelled"}
            if allowed:
                update["rawOutput"] = "synthetic allowed content"
            self.record("session/update", update)
            self.notification("session/update", {"sessionId": SESSION, "_meta": {"promptId": self.turn}, "update": update})
            if allowed:
                self.text((self.project / "allow.txt").read_text())
            self.finish("end_turn" if allowed else "cancelled", None if allowed else "PermissionRejected")

    def receive(self, deadline):
        return self.messages.get_nowait()


class ConfigAuditTests(unittest.TestCase):
    def test_original_configuration_is_unchanged(self):
        audit = probe.audit_native_settings(probe.configuration())
        self.assertTrue(audit["settings_scope_verified"])
        self.assertTrue(audit["bytes_unchanged"])
        self.assertEqual(audit["native_initialization_state"], "unchanged")

    def test_exact_34_purge_marker_is_distinct_from_full_initialization(self):
        after = probe.configuration() + b'\n[marketplace]\ndefault_skills_installs_purged = true\n'
        audit = probe.audit_native_settings(after)
        self.assertTrue(audit["settings_scope_verified"])
        self.assertTrue(audit["permission_section_unchanged"])
        self.assertFalse(audit["bytes_unchanged"])
        self.assertTrue(audit["native_marketplace_purge_only"])
        self.assertFalse(audit["native_marketplace_initialization_only"])
        self.assertEqual(audit["native_initialization_state"], "purge_only_1_0_34")
        self.assertEqual(audit["after_sha256"], probe.sha(after))

    def test_previous_complete_initialization_remains_accepted(self):
        after = probe.configuration() + (
            b'\n[marketplace]\ndefault_skills_installs_purged = true\n'
            b'official_marketplace_auto_installed = true\n'
            b'[[marketplace.sources]]\nname = "xAI Official"\n'
            b'git = "https://github.com/xai-org/plugin-marketplace.git"\n')
        audit = probe.audit_native_settings(after)
        self.assertTrue(audit["settings_scope_verified"])
        self.assertTrue(audit["native_marketplace_initialization_only"])
        self.assertFalse(audit["native_marketplace_purge_only"])
        self.assertEqual(audit["native_initialization_state"], "full_marketplace")

    def test_unknown_partial_or_wrong_type_marker_is_rejected(self):
        for addition in (
            b'[marketplace]\ndefault_skills_installs_purged = true\nunknown = "private value"\n',
            b'[marketplace]\ndefault_skills_installs_purged = true\nofficial_marketplace_auto_installed = true\n',
            b'[marketplace]\ndefault_skills_installs_purged = 1\n',
            b'[marketplace]\ndefault_skills_installs_purged = false\n',
            b'[unknown]\nprivate = "private value"\n',
            b'[marketplace]\ndefault_skills_installs_purged = true\ninvalid toml'):
            with self.subTest(addition=addition):
                audit = probe.audit_native_settings(probe.configuration() + addition)
                self.assertFalse(audit["settings_scope_verified"])
                self.assertEqual(audit["native_initialization_state"], "unverified")
                self.assertNotIn("private value", json.dumps(audit))

    def test_purge_marker_never_masks_permission_or_other_config_changes(self):
        for before in (probe.configuration().replace(b'action = "ask"', b'action = "allow"'),
                       probe.configuration().replace(b'auto_update = false', b'auto_update = true')):
            audit = probe.audit_native_settings(before + b'[marketplace]\ndefault_skills_installs_purged = true\n')
            self.assertFalse(audit["settings_scope_verified"])
            self.assertFalse(audit["native_marketplace_purge_only"])
            self.assertEqual(audit["native_initialization_state"], "unverified")


class ProtocolTests(unittest.TestCase):
    def test_read_marker_allows_formatting_but_requires_one_complete_unique_value(self):
        marker = "a" * 64
        self.assertTrue(probe.contains_unique_read_marker("文件内容：\n```\n" + marker + "\n```", marker))
        for output in ["b" * 64, marker + "0", "x" + marker, marker + " " + marker,
                       marker + " " + "b" * 64, marker[:63], marker + "汉" * 700]:
            with self.subTest(output_bytes=len(output.encode())):
                self.assertFalse(probe.contains_unique_read_marker(output, marker))

    def test_native_load_replay_cannot_finish_new_input_or_change_session(self):
        frame = {"jsonrpc": "2.0", "method": "_x.ai/session/update", "params": {
            "sessionId": SESSION, "_meta": {"isReplay": True, "eventId": "old-event"},
            "update": {"sessionUpdate": "turn_completed", "prompt_id": "old-turn", "stop_reason": "end_turn"}}}
        ledger = probe.Ledger(Path("."), {"cases": []}, time.monotonic() + 5)
        ledger.session = SESSION
        ledger.pending[4] = "session/load"
        before = copy.deepcopy({key: item for key, item in vars(ledger).items()
                                if key not in {"evidence", "config_guard"}})
        wire = mock.Mock()
        wire.receive.side_effect = [frame, TimeoutError()]
        with self.assertRaises(TimeoutError):
            ledger.receive(wire, 4)
        self.assertEqual(before, {key: item for key, item in vars(ledger).items()
                                 if key not in {"evidence", "config_guard"}})
        self.assertEqual(ledger.evidence, {"cases": []})
        for params, reason in (({**frame["params"], "sessionId": OTHER_SESSION}, "notification_session"),
                               ({**frame["params"], "_meta": {}}, "history_alias_without_replay")):
            with self.assertRaisesRegex(probe.Rejected, reason):
                ledger.notification(mock.Mock(), {**frame, "params": params})
        ledger.phase = probe.PHASES[3]
        with self.assertRaisesRegex(probe.Rejected, "replay_during_new_input"):
            ledger.notification(mock.Mock(), frame)

    def test_nonterminal_extensions_cannot_grant_permissions_or_finish_a_turn(self):
        frame = {"jsonrpc": "2.0", "method": "_x.ai/session_notification", "params": {
            "sessionId": SESSION, "update": {"sessionUpdate": "future_metadata", "toolCallId": "tool-other",
                "permission": "allow", "stopReason": "end_turn", "content": "private-extension"}}}
        for phase in (None, *probe.PHASES):
            ledger = probe.Ledger(Path("."), {"cases": []}, time.monotonic() + 5)
            ledger.phase, ledger.session = phase, SESSION
            ledger.pending[4] = "session/prompt"
            before = copy.deepcopy({key: item for key, item in vars(ledger).items()
                                    if key not in {"evidence", "config_guard"}})
            wire = mock.Mock()
            wire.receive.side_effect = [frame, TimeoutError()]
            with self.assertRaises(TimeoutError):
                ledger.receive(wire, 4)
            self.assertEqual(before, {key: item for key, item in vars(ledger).items()
                                     if key not in {"evidence", "config_guard"}})
            self.assertEqual(ledger.evidence["cases"], [])
            self.assertEqual(ledger.evidence["uninterpreted_session_metadata"], 1)
            self.assertNotIn("private-extension", json.dumps(ledger.evidence))
            wire.send.assert_not_called()

    def test_nonterminal_extensions_keep_reverse_request_and_session_guards(self):
        value = {"jsonrpc": "2.0", "method": "_x.ai/session_notification", "params": {
            "sessionId": SESSION, "update": {"sessionUpdate": "future_metadata"}}}
        ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
        ledger.phase, ledger.session = probe.PHASES[0], SESSION
        wire = mock.Mock()
        with self.assertRaisesRegex(probe.Rejected, "unexpected_reverse_request"):
            ledger.notification(wire, {**value, "id": 1})
        self.assertEqual(wire.send.call_args.args[0]["error"]["code"], -32601)
        for params, reason in (({**value["params"], "sessionId": OTHER_SESSION}, "notification_session"),
                               ({**value["params"], "_meta": {"isReplay": True}}, "replay_during_new_input")):
            with self.assertRaisesRegex(probe.Rejected, reason):
                ledger.notification(mock.Mock(), {**value, "params": params})
        tagged = {**value, "params": {**value["params"], "_meta": {"eventId": "same-event"}}}
        ledger.notification(mock.Mock(), tagged)
        with self.assertRaisesRegex(probe.Rejected, "duplicate_native_event"):
            ledger.notification(mock.Mock(), tagged)
        with self.assertRaisesRegex(probe.Rejected, "update_turn_mismatch"):
            ledger.notification(mock.Mock(), {**value, "method": "session/update"})

    def test_internal_reload_cannot_consume_pending_rpc_or_complete_a_phase(self):
        frame = {"jsonrpc": "2.0", "id": "skills-reload", "result": {"result": {"reloaded": 1}}}
        for phase in (None, *probe.PHASES):
            ledger = probe.Ledger(Path("."), {"cases": []}, time.monotonic() + 5)
            ledger.phase = phase
            ledger.pending[4] = "session/prompt"
            before = copy.deepcopy({key: item for key, item in vars(ledger).items()
                                    if key not in {"evidence", "config_guard"}})
            wire = mock.Mock()
            wire.receive.side_effect = [frame, frame, TimeoutError()]
            with self.assertRaises(TimeoutError):
                ledger.receive(wire, 4)
            self.assertEqual(before, {key: item for key, item in vars(ledger).items()
                                     if key not in {"evidence", "config_guard"}})
            self.assertEqual(ledger.evidence, {"cases": [], "internal_reload_responses": 2})
            wire.send.assert_not_called()
        for count in (0, 2**64 - 1):
            self.assertTrue(probe.internal_skills_reload_success({**frame, "result": {"result": {"reloaded": count}}}))
        invalid = [{**frame, "id": 4}, {**frame, "id": "skills-reload "}, {**frame, "error": {}},
                   {**frame, "method": "session/request_permission"}, {**frame, "result": {"reloaded": 1}},
                   {**frame, "result": {"result": {"reloaded": 1, "status": "completed"}}}]
        invalid += [{**frame, "result": {"result": {"reloaded": count}}} for count in (True, -1, 2**64, "1", None)]
        for value in invalid:
            self.assertFalse(probe.internal_skills_reload_success(value))

    def test_native_session_title_is_not_turn_output_or_completion(self):
        value = {"jsonrpc": "2.0", "method": "_x.ai/session_notification", "params": {
            "sessionId": SESSION, "update": {"sessionUpdate": "session_summary_generated",
                "session_summary": "private-title"}}}
        standard = {"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": SESSION, "update": {"sessionUpdate": "session_info_update", "title": "private-title"}}}
        for phase in (None, *probe.PHASES):
            ledger = probe.Ledger(Path("."), {"cases": []}, time.monotonic() + 5)
            ledger.phase, ledger.session = phase, SESSION
            ledger.pending[4] = "session/prompt"
            before = copy.deepcopy({key: item for key, item in vars(ledger).items()
                                    if key not in {"evidence", "config_guard"}})
            wire = mock.Mock()
            wire.receive.side_effect = [value, standard, TimeoutError()]
            with self.assertRaises(TimeoutError):
                ledger.receive(wire, 4)
            self.assertEqual(before, {key: item for key, item in vars(ledger).items()
                                     if key not in {"evidence", "config_guard"}})
            self.assertEqual(ledger.evidence, {"cases": []})
            wire.send.assert_not_called()
        with self.assertRaisesRegex(probe.Rejected, "notification_session"):
            ledger.notification(mock.Mock(), {**value, "params": {**value["params"], "sessionId": OTHER_SESSION}})
        with self.assertRaisesRegex(probe.Rejected, "notification_session"):
            ledger.notification(mock.Mock(), {**standard, "params": {**standard["params"], "sessionId": OTHER_SESSION}})

    def test_announcements_are_inert_and_cannot_answer_requests_or_complete_turns(self):
        value = {"jsonrpc": "2.0", "method": "_x.ai/announcements/update", "params": {
            "announcements": [{"text": "private-announcement", "sessionId": OTHER_SESSION,
                               "promptId": "old-turn", "status": "completed"}]}}
        for phase in (None, *probe.PHASES):
            ledger = probe.Ledger(Path("."), {"cases": []}, time.monotonic() + 5)
            ledger.phase = phase
            ledger.pending[4] = "session/prompt"
            before = copy.deepcopy({key: item for key, item in vars(ledger).items()
                                    if key not in {"evidence", "config_guard"}})
            wire = mock.Mock()
            wire.receive.side_effect = [value, TimeoutError()]
            with self.assertRaises(TimeoutError):
                ledger.receive(wire, 4)
            self.assertEqual(before, {key: item for key, item in vars(ledger).items()
                                     if key not in {"evidence", "config_guard"}})
            self.assertEqual(ledger.evidence["announcement_notifications"], 1)
            self.assertNotIn("private-announcement", json.dumps(ledger.evidence))
            wire.send.assert_not_called()
        for extra in ({"id": 1}, {"id": None}, {"params": []}, {"error": {}},
                      {"result": {}}, {"method": "_x.ai/announcements/other"}):
            ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
            with self.assertRaises(probe.Rejected):
                ledger.notification(mock.Mock(), {**value, **extra})
            self.assertNotIn("announcement_notifications", ledger.evidence)

    def test_settings_notification_never_changes_permission_or_lifecycle(self):
        for phase in (None, *probe.PHASES):
            ledger = probe.Ledger(Path("."), {"cases": []}, time.monotonic() + 5)
            ledger.phase = phase
            ledger.session = SESSION if phase else None
            ledger.pending[4] = "session/prompt"
            before = copy.deepcopy({key: value for key, value in vars(ledger).items()
                                    if key not in {"evidence", "config_guard"}})
            params = {"permission_mode": "auto", "allow_access": True,
                      "gate_message": "private-gate-body", "sessionId": OTHER_SESSION,
                      "promptId": "other-turn", "status": "completed"}
            wire = mock.Mock()
            wire.receive.side_effect = [{"jsonrpc": "2.0", "method": "_x.ai/settings/update", "params": params},
                                        TimeoutError()]
            with self.assertRaises(TimeoutError):
                ledger.receive(wire, 4)
            self.assertEqual(before, {key: value for key, value in vars(ledger).items()
                                     if key not in {"evidence", "config_guard"}})
            self.assertEqual(ledger.evidence["settings_notifications"], 1)
            self.assertEqual(ledger.evidence["last_settings_params"], probe.summary(params))
            self.assertNotIn("private-gate-body", json.dumps(ledger.evidence))
            wire.send.assert_not_called()

    def test_settings_broadcast_is_not_a_reverse_request_or_arbitrary_namespace(self):
        value = {"jsonrpc": "2.0", "method": "_x.ai/settings/update", "params": {}}
        for request_id in (1, "reverse-request", None):
            ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
            wire = mock.Mock()
            with self.assertRaisesRegex(probe.Rejected, "unexpected_reverse_request"):
                ledger.notification(wire, {**value, "id": request_id})
            self.assertEqual(wire.send.call_args.args[0]["error"]["code"], -32601)
            self.assertNotIn("settings_notifications", ledger.evidence)
        for extra in ({"params": []}, {"result": {}}, {"error": {}}, {"method": "_x.ai/settings/other"}):
            ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
            with self.assertRaises(probe.Rejected):
                ledger.notification(mock.Mock(), {**value, **extra})
            self.assertNotIn("settings_notifications", ledger.evidence)

    def test_session_roster_notification_cannot_advance_any_phase_or_identity(self):
        for phase in (None, *probe.PHASES):
            with self.subTest(phase=phase):
                evidence = {"cases": []}
                ledger = probe.Ledger(Path("."), evidence, time.monotonic() + 5)
                ledger.phase = phase
                ledger.session = SESSION if phase else None
                ledger.pending[4] = "session/prompt"
                before = copy.deepcopy({key: value for key, value in vars(ledger).items()
                                        if key not in {"evidence", "config_guard"}})
                notification = {"jsonrpc": "2.0", "method": "_x.ai/sessions/changed", "params": {
                    "roster": [{"sessionId": OTHER_SESSION, "promptId": "other-turn",
                                "status": "completed", "text": "private-roster-body"}], "removed": []}}
                wire = mock.Mock()
                wire.receive.side_effect = [notification, TimeoutError()]
                with self.assertRaises(TimeoutError):
                    ledger.receive(wire, 4)
                self.assertEqual(before, {key: value for key, value in vars(ledger).items()
                                         if key not in {"evidence", "config_guard"}})
                self.assertEqual(evidence["session_roster_notifications"], 1)
                self.assertEqual(evidence["last_session_roster_params"], probe.summary(notification["params"]))
                self.assertEqual(evidence["cases"], [])
                self.assertNotIn("private-roster-body", json.dumps(evidence))
                wire.send.assert_not_called()

    def test_session_roster_reverse_request_and_invalid_envelopes_are_rejected(self):
        value = {"jsonrpc": "2.0", "method": "_x.ai/sessions/changed", "params": {}}
        for request_id in (1, "reverse-request", None):
            ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
            wire = mock.Mock()
            with self.assertRaisesRegex(probe.Rejected, "unexpected_reverse_request"):
                ledger.notification(wire, {**value, "id": request_id})
            self.assertEqual(wire.send.call_args.args[0]["error"]["code"], -32601)
            self.assertNotIn("session_roster_notifications", ledger.evidence)
        for extra in ({"params": []}, {"result": {}}, {"error": {}}, {"method": "_x.ai/sessions/other"}):
            ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
            with self.assertRaises(probe.Rejected):
                ledger.notification(mock.Mock(), {**value, **extra})
            self.assertNotIn("session_roster_notifications", ledger.evidence)

    def test_safe_method_diagnostic_does_not_authorize_unknown_notifications(self):
        for method in ("_x.ai/sessions/changed", "_x.ai/session/catalog_changed"):
            value = {"method": method, "params": {"method": "_x.ai/private_text", "text": "private-body"}}
            shape = probe.diagnostic_shape(value)
            self.assertEqual(shape["safe_method_name"], method)
            self.assertNotIn("_x.ai/private_text", json.dumps(shape))
            self.assertNotIn("private-body", json.dumps(shape))
        for method in ("_x.ai/private body", "_x.ai/" + "a" * 33, "_x.ai/a/b/c/d/e",
                       "_x.ai/a?token=secret", "https://auth.x.ai/a", "_x.ai/a\nsecret", [], None):
            self.assertNotIn("safe_method_name", probe.diagnostic_shape({"method": method}))
        ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
        wire = mock.Mock()
        wire.receive.return_value = {"jsonrpc": "2.0", "method": "_x.ai/session/catalog_changed", "params": {}}
        with self.assertRaisesRegex(probe.Rejected, "unknown_native_notification"):
            ledger.receive(wire, 1)
        self.assertEqual(ledger.evidence["last_frame_shape"]["safe_method_name"], "_x.ai/session/catalog_changed")

    def test_json_duplicate_keys_and_nonfinite_values_are_rejected(self):
        for value in (b'{"id":1,"id":2}', b'{"x":NaN}', b'{"x":Infinity}'):
            with self.subTest(value=value), self.assertRaises(probe.Rejected):
                probe.parse(value)

    def test_history_requires_same_turn_monotonic_events_and_real_completion(self):
        value = history()
        result = probe.history_snapshot(value, SESSION, "turn", "end_turn")
        self.assertEqual(result["output"], "answer")
        self.assertIsNotNone(result["watermark"])
        self.assertIsNone(probe.history_snapshot(value, SESSION, "old-turn", "end_turn")["watermark"])
        for mutation in (lambda x: x.update(hasMore=True), lambda x: x.update(totalCount=True),
                         lambda x: x.update(lastEventId=SESSION + "-1"),
                         lambda x: x["updates"][1]["params"]["_meta"].update(eventId=SESSION + "-1"),
                         lambda x: x["updates"][1]["params"].update(sessionId=OTHER_SESSION),
                         lambda x: x["updates"][1]["params"]["update"].update(stop_reason="cancelled")):
            changed = copy.deepcopy(value)
            mutation(changed)
            with self.assertRaises(probe.Rejected):
                probe.history_snapshot(changed, SESSION, "turn", "end_turn")

    def test_cancelled_history_keeps_real_text_and_rejects_post_terminal_text(self):
        value = history("审批前文字", reason="cancelled", category="PermissionRejected")
        result = probe.history_snapshot(value, SESSION, "turn", "cancelled")
        self.assertEqual(result["output"], "审批前文字")
        self.assertEqual(result["category"], "PermissionRejected")
        extra = copy.deepcopy(value["updates"][0])
        extra["params"]["_meta"]["eventId"] = SESSION + "-3"
        value["updates"].append(extra)
        value.update(totalCount=3, lastEventId=SESSION + "-3")
        with self.assertRaises(probe.Rejected):
            probe.history_snapshot(value, SESSION, "turn", "cancelled")

    def test_history_orphan_tool_update_is_not_no_tools(self):
        value = history()
        value["updates"][0]["params"]["update"] = {"sessionUpdate": "tool_call_update", "toolCallId": "other", "status": "completed"}
        with self.assertRaises(probe.Rejected):
            probe.history_snapshot(value, SESSION, "turn", "end_turn")

    def test_unknown_old_and_duplicate_rpc_responses_fail(self):
        for message in ({"jsonrpc": "2.0", "id": 9, "result": {}},
                        {"jsonrpc": "2.0", "id": True, "result": {}},
                        {"jsonrpc": "2.0", "id": 1, "error": {"message": "private text"}},
                        {"jsonrpc": "2.0", "method": "_x.ai/unknown", "params": {}}):
            ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
            ledger.pending[1] = "initialize"
            wire = mock.Mock()
            wire.receive.return_value = message
            with self.assertRaises(probe.Rejected):
                ledger.receive(wire, 1)
            self.assertNotIn("private text", json.dumps(ledger.evidence))
        ledger.pending[1] = "initialize"
        ledger.responses.add(1)
        wire.receive.return_value = {"jsonrpc": "2.0", "id": 1, "result": {}}
        with self.assertRaises(probe.Rejected):
            ledger.receive(wire, 1)

    def test_extra_tool_and_wrong_session_fail_without_dispatch(self):
        ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
        ledger.session, ledger.turn, ledger.phase = SESSION, "turn", probe.PHASES[2]
        for session in (SESSION, OTHER_SESSION):
            wire = mock.Mock()
            message = {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session,
                "_meta": {"promptId": "turn"}, "update": {"sessionUpdate": "tool_call", "toolCallId": "call"}}}
            with self.assertRaises(probe.Rejected):
                ledger.notification(wire, message)
            wire.send.assert_not_called()

    def test_network_budget_signal_fails_immediately_even_below_byte_limit(self):
        tunnel = mock.Mock(lock=threading.Lock(), forwarded=9, bytes=100,
                           events=[{"event": "tunnel_byte_budget_exhausted"}], deadline=time.monotonic() + 10)
        with self.assertRaises(probe.Rejected):
            probe.network_guard(tunnel)
        tunnel.events = []
        probe.network_guard(tunnel)

    def test_request_budget_and_methods_are_closed(self):
        ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
        wire = mock.Mock()
        with self.assertRaises(probe.Rejected):
            ledger.request(wire, "session/resume", {})
        ledger.requests = probe.MAX_REQUESTS
        with self.assertRaises(probe.Rejected):
            ledger.request(wire, "initialize", {})
        wire.send.assert_not_called()

    def test_missing_run_confirmation_stops_before_manifest_or_auth(self):
        for confirmation, env in ((None, "1"), (probe.CONFIRM, "0"), ("other", "1")):
            with mock.patch.dict(os.environ, {probe.RUN_ENV: env}), \
                    mock.patch.object(probe, "validate_manifest") as validate, \
                    mock.patch.object(probe.official, "copy_private_auth") as auth, self.assertRaises(probe.Rejected):
                probe.run(Path("/fixture"), Path("/grok"), Path("/auth"), confirmation)
            validate.assert_not_called()
            auth.assert_not_called()

    def test_main_defaults_to_prepare_and_failed_run_propagates_nonzero(self):
        argv = ["--fixture-root", "/fixture", "--grok", "/grok", "--official-grok-home", "/auth"]
        with mock.patch.object(probe, "prepare", return_value={"prepared_only": True}) as prepare, \
                mock.patch.object(probe, "run") as run, mock.patch("builtins.print"):
            self.assertEqual(probe.main(argv), 0)
        prepare.assert_called_once()
        run.assert_not_called()
        with mock.patch.object(probe, "run", return_value={"passed": False}), mock.patch("builtins.print"):
            self.assertEqual(probe.main(argv + ["--run", "--confirm", probe.CONFIRM]), 1)

    def test_safe_shape_includes_known_protocol_paths_without_arbitrary_strings(self):
        value = {"method": "session/update", "params": {"update": {"sessionUpdate": "tool_call",
            "content": {"text": "private-secret-body"}}, "private-secret-key": "private-secret-value"}}
        diagnostic = probe.diagnostic_shape(value)
        rendered = json.dumps(diagnostic)
        self.assertNotIn("private-secret", rendered)
        self.assertIn("params.update.content.text", rendered)
        self.assertEqual(diagnostic["known_enums"]["method"], "session/update")
        self.assertEqual(diagnostic["unknown_keys"], 1)
        self.assertLessEqual(len(diagnostic["known_paths"]), 64)

    def test_eof_drain_rejects_late_rpc_and_unknown_notifications(self):
        ledger = probe.Ledger(Path("."), {}, time.monotonic() + 5)
        for value in ({"jsonrpc": "2.0", "id": 1, "result": {}},
                      {"jsonrpc": "2.0", "method": "unknown", "params": {}}):
            wire = mock.Mock(messages=queue.Queue())
            wire.messages.put(value)
            with self.assertRaises(probe.Rejected):
                ledger.drain(wire)

    def test_real_python_main_entrypoint_propagates_mock_failure(self):
        code = '''import grok_native_p0_probe as p
p.run=lambda *args:{'passed':False}
raise SystemExit(p.main(['--fixture-root','/fixture','--grok','/grok','--official-grok-home','/auth','--run','--confirm',p.CONFIRM]))
'''
        env = {key: value for key, value in os.environ.items() if key.upper() in {"PATH", "SYSTEMROOT", "WINDIR"}}
        env["PYTHONPATH"] = str(Path(probe.__file__).parent)
        result = subprocess.run([sys.executable, "-B", "-c", code], capture_output=True, env=env, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(json.loads(result.stdout), {"passed": False})


@unittest.skipUnless(os.name == "posix", "私有 macOS 原生夹具依赖 POSIX 文件所有权和进程组")
class PrivateFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / "fixture"
        self.auth = self.base / "auth"
        self.auth.mkdir(mode=0o700)
        self.binary = self.base / "grok"
        self.binary.write_bytes(b"synthetic, never execute")
        self.binary.chmod(0o600)
        self.binary_patch = mock.patch.object(probe, "validate_binary", return_value={"sha256": "0" * 64})
        self.binary_patch.start()
        probe.prepare(self.root, self.binary, self.auth)

    def tearDown(self):
        self.binary_patch.stop()
        self.temporary.cleanup()

    def test_prepare_never_copies_auth_or_runs_subprocess_and_manifest_is_private(self):
        self.assertFalse((self.root / "home/.grok/auth.json").exists())
        self.assertEqual((self.root / "manifest.json").stat().st_mode & 0o777, 0o600)
        with mock.patch.object(probe.subprocess, "Popen") as spawn, \
                mock.patch.object(probe.official, "copy_private_auth") as auth:
            probe.prepare(self.base / "another", self.binary, self.auth)
        spawn.assert_not_called()
        auth.assert_not_called()
        probe.validate_manifest(self.root, self.binary, self.auth)

    def test_modified_manifest_sentinel_and_directory_reuse_are_rejected(self):
        with self.assertRaises(probe.Rejected):
            probe.prepare(self.root, self.binary, self.auth)
        path = self.root / "manifest.json"
        original = path.read_bytes()
        value = probe.parse(original)
        value["max_native_inputs"] = 5
        path.write_bytes(probe.encoded(value))
        with self.assertRaises(probe.Rejected):
            probe.validate_manifest(self.root, self.binary, self.auth)
        path.write_bytes(original)
        (self.root / "project/allow.txt").write_bytes(b"changed")
        with self.assertRaises(probe.Rejected):
            probe.validate_manifest(self.root, self.binary, self.auth)

    def test_rejected_config_audit_is_retained_without_native_content(self):
        manifest = probe.validate_manifest(self.root, self.binary, self.auth)
        after = probe.configuration().replace(b'action = "ask"', b'action = "allow"')
        (self.root / "home/.grok/config.toml").write_bytes(after)
        evidence = {}
        with self.assertRaisesRegex(probe.Rejected, "native_config_changed"):
            probe.verify_fixture(self.root, manifest, evidence)
        self.assertFalse(evidence["config_audit"]["permission_section_unchanged"])
        self.assertFalse(evidence["config_audit"]["settings_scope_verified"])
        self.assertEqual(evidence["config_audit"]["after_sha256"], probe.sha(after))
        self.assertNotIn('action = "allow"', json.dumps(evidence))

    def test_symlink_and_nonprivate_manifest_are_rejected(self):
        path = self.root / "manifest.json"
        path.chmod(0o644)
        with self.assertRaises(probe.Rejected):
            probe.validate_manifest(self.root, self.binary, self.auth)
        path.chmod(0o600)
        link = self.base / "alias"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(probe.Rejected):
            probe.private_directory(link)

    def test_every_prepared_directory_is_private_and_environment_cannot_follow_escape(self):
        for relative in probe.PRIVATE_DIRECTORIES:
            probe.private_directory(self.root / relative)
            self.assertEqual((self.root / relative).stat().st_mode & 0o777, 0o700)
        env = probe.environment(self.root, 1234)
        for key in ("HOME", "GROK_HOME", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "XDG_CONFIG_HOME", "TMPDIR"):
            self.assertTrue(Path(env[key]).resolve().is_relative_to(self.root))
        config = self.root / "home/.config"
        config.rmdir()
        config.symlink_to(self.auth, target_is_directory=True)
        with self.assertRaises(probe.Rejected):
            probe.environment(self.root, 1234)
        with self.assertRaises(probe.Rejected):
            probe.validate_manifest(self.root, self.binary, self.auth)

    def test_permission_accepts_only_exact_read_and_exact_one_time_choices(self):
        target = self.root / "project/allow.txt"
        value = permission(target)
        check = lambda candidate: probe.exact_permission(candidate, SESSION, "turn", "call", target, target.parent)
        self.assertTrue(check(value))
        mutations = (
            lambda x: x.update(id=True),
            lambda x: x["params"].update(sessionId=OTHER_SESSION),
            lambda x: x["params"]["toolCall"].update(toolCallId="old-call"),
            lambda x: x["params"]["toolCall"]["rawInput"].update(target_file=str(target.parent / "deny.txt")),
            lambda x: x["params"]["toolCall"]["rawInput"].update(offset=True),
            lambda x: x["params"]["toolCall"]["rawInput"].update(unexpected="private-body"),
            lambda x: x["params"]["toolCall"]["_meta"]["x.ai/tool"].update(version=True),
            lambda x: x["params"]["toolCall"]["_meta"]["x.ai/tool"].update(namespace="opencode"),
            lambda x: x["params"]["options"][0].update(optionId="enable-always-approve"),
            lambda x: x["params"]["options"].append(copy.deepcopy(x["params"]["options"][0])),
        )
        for mutation in mutations:
            candidate = copy.deepcopy(value)
            mutation(candidate)
            self.assertFalse(check(candidate))
            diagnostic = probe.permission_summary(candidate, SESSION, "turn", "call", target, target.parent)
            self.assertNotIn("private-body", json.dumps(diagnostic))

    def test_unknown_and_repeated_permission_is_cancelled_not_allowed(self):
        ledger = probe.Ledger(self.root, {}, time.monotonic() + 5)
        ledger.session, ledger.turn, ledger.call, ledger.phase = SESSION, "turn", "call", probe.PHASES[0]
        wire = mock.Mock()
        value = permission(self.root / "project/allow.txt")
        ledger.notification(wire, value)
        self.assertEqual(wire.send.call_args.args[0]["result"]["outcome"]["optionId"], "allow-once")
        with self.assertRaises(probe.Rejected):
            ledger.notification(wire, value)
        self.assertEqual(wire.send.call_args.args[0]["result"]["outcome"], {"outcome": "cancelled"})

    def test_four_input_chain_uses_cold_same_history_and_keeps_denied_prefix(self):
        evidence = {"cases": []}
        ledger = probe.Ledger(self.root, evidence, time.monotonic() + 5)
        secret = (self.root / "project/allow.txt").read_text()
        first = FakeWire(self.root / "project")
        self.assertEqual(ledger.open(first), SESSION)
        first_turn = ledger.run_phase(first, probe.PHASES[0], "read allow", secret)
        ledger.run_phase(first, probe.PHASES[1], "read deny", secret)
        ledger.run_phase(first, probe.PHASES[2], "output numbers", secret)
        self.assertEqual(evidence["cases"][1]["output_sha256"], probe.sha("等待审批。".encode()))
        second = FakeWire(self.root / "project", saved=first.saved)
        self.assertEqual(ledger.open(second, SESSION), SESSION)
        restored = ledger.rpc(second, "_x.ai/session/updates", {"sessionId": SESSION})
        self.assertEqual(probe.history_snapshot(restored, SESSION, first_turn, "end_turn")["output"], secret)
        ledger.run_phase(second, probe.PHASES[3], "recall first line", secret)
        self.assertEqual(ledger.inputs, 4)
        self.assertEqual(len(evidence["cases"]), 4)
        self.assertEqual(len(list((self.root / "cases").iterdir())), 4)
        self.assertTrue(all(secret not in probe.encoded(frame).decode() for frame in first.sent + second.sent))
        with self.assertRaises(probe.Rejected):
            ledger.run_phase(second, probe.PHASES[3], "repeat", secret)

    def test_read_result_accepts_formatting_without_exposing_marker_in_safe_evidence(self):
        evidence = {"cases": []}
        ledger = probe.Ledger(self.root, evidence, time.monotonic() + 5)
        wire = FakeWire(self.root / "project")
        ledger.open(wire)
        original = wire.text
        wire.text = lambda text: original("读取结果：\n`" + text + "`")
        secret = (self.root / "project/allow.txt").read_text()
        ledger.run_phase(wire, probe.PHASES[0], "read allow", secret)
        self.assertTrue(evidence["last_turn_result"]["tool_completed"])
        self.assertFalse(evidence["last_turn_result"]["exact_read_marker"])
        self.assertTrue(evidence["last_turn_result"]["unique_read_marker"])
        self.assertEqual(len(evidence["cases"]), 1)
        self.assertNotIn(secret, json.dumps(evidence))

    def test_returned_marker_cannot_hide_failed_tool_in_persisted_history(self):
        evidence = {"cases": []}
        ledger = probe.Ledger(self.root, evidence, time.monotonic() + 5)
        wire = FakeWire(self.root / "project")
        ledger.open(wire)
        original = wire.record
        def record(method, update, category=None):
            update = copy.deepcopy(update)
            if update.get("status") == "completed":
                update["status"] = "failed"
            original(method, update, category)
        wire.record = record
        with self.assertRaisesRegex(probe.Rejected, "^allowed_read_tool_not_completed$"):
            ledger.run_phase(wire, probe.PHASES[0], "read allow", (self.root / "project/allow.txt").read_text())
        self.assertFalse(evidence["last_turn_result"]["tool_completed"])
        self.assertEqual(evidence["cases"], [])

    def test_wrong_cold_recall_fails_even_with_real_completed_history(self):
        evidence = {"cases": []}
        ledger = probe.Ledger(self.root, evidence, time.monotonic() + 5)
        ledger.inputs = ledger.written_inputs = 3
        wire = FakeWire(self.root / "project", saved=[])
        ledger.open(wire, SESSION)
        original_text = wire.text
        wire.text = lambda text: original_text("wrong remembered value")
        with self.assertRaises(probe.Rejected):
            ledger.run_phase(wire, probe.PHASES[3], "recall first line", (self.root / "project/allow.txt").read_text())
        self.assertEqual(evidence["cases"], [])

    def test_complete_run_mock_keeps_product_flags_false_and_removes_auth(self):
        saved = []
        created = []
        tunnel = mock.Mock(lock=threading.Lock(), forwarded=9, bytes=1000, events=[], deadline=time.monotonic() + 10)
        tunnel.start.return_value = 1234
        tunnel.close.return_value = True
        def copy_auth(source, target):
            probe.create_file(target / "auth.json", b"synthetic opaque bytes")
        def spawn(argv, env, directory, guard):
            wire = FakeWire(directory, saved=saved if created else None)
            wire.close = lambda graceful: True
            if not created:
                wire.saved = saved
                (self.root / "home/.grok/config.toml").write_bytes(probe.configuration()
                    + b'[marketplace]\ndefault_skills_installs_purged = true\n')
            created.append(wire)
            return wire
        with mock.patch.object(probe.sys, "platform", "darwin"), mock.patch.dict(os.environ, {probe.RUN_ENV: "1"}), \
                mock.patch.object(probe.fixed, "verify_version", return_value=probe.fixed.VERSION_OUTPUTS[probe.VERSION]), \
                mock.patch.object(probe.official, "OfficialTunnel", return_value=tunnel), \
                mock.patch.object(probe, "sandbox_canary"), mock.patch.object(probe.official, "copy_private_auth", side_effect=copy_auth), \
                mock.patch.object(probe, "Wire", side_effect=spawn):
            result = probe.run(self.root, self.binary, self.auth, probe.CONFIRM)
        self.assertTrue(result["passed"], result)
        self.assertEqual(result["model_inputs_sent"], 4)
        self.assertEqual(result["model_inputs_claimed"], 4)
        self.assertEqual(result["native_session_processes"], 2)
        self.assertEqual(result["normal_eof_count"], 2)
        self.assertTrue(result["auth_copy_removed"])
        self.assertTrue(result["candidate_unverified"])
        self.assertFalse(result["production_runtime_verified"])
        self.assertFalse(result["full_cli_parity_acceptance_passed"])
        self.assertEqual(result["config_audit"]["native_initialization_state"], "purge_only_1_0_34")
        self.assertFalse(result["config_audit"]["bytes_unchanged"])
        serialized = (self.root / "result.safe.json").read_text()
        self.assertNotIn((self.root / "project/allow.txt").read_text(), serialized)
        self.assertNotIn((self.root / "project/deny.txt").read_text(), serialized)
        self.assertNotIn("synthetic opaque bytes", serialized)
        self.assertFalse((self.root / "home/.grok/auth.json").exists())

    def test_cancel_notification_alone_and_reused_case_are_not_success(self):
        ledger = probe.Ledger(self.root, {"cases": []}, time.monotonic() + 5)
        wire = FakeWire(self.root / "project")
        ledger.open(wire)
        ledger.inputs = wire.input = 2
        original = wire.send
        def send(value):
            if value.get("method") == "session/cancel":
                wire.sent.append(value)
                wire.finish("end_turn")
            else:
                original(value)
        wire.send = send
        with self.assertRaises(probe.Rejected):
            ledger.run_phase(wire, probe.PHASES[2], "numbers", (self.root / "project/allow.txt").read_text())
        self.assertFalse(ledger.evidence["cases"])
        ledger.pending.clear()
        ledger.inputs = 2
        with self.assertRaises(FileExistsError):
            ledger.request(wire, "session/prompt", {})

    def test_cleanup_tunnel_exception_still_removes_private_auth(self):
        auth = self.root / "home/.grok/auth.json"
        probe.create_file(auth, b"synthetic opaque bytes")
        tunnel = mock.Mock()
        tunnel.close.side_effect = RuntimeError("synthetic failure")
        evidence = {}
        self.assertFalse(probe.cleanup(None, tunnel, auth, evidence))
        self.assertFalse(auth.exists())
        self.assertTrue(evidence["auth_copy_removed"])
        self.assertFalse(evidence["cleanup_confirmed"])

    def test_preexisting_result_cannot_trigger_auth_copy_or_native_run(self):
        probe.create_file(self.root / "result.safe.json", b"preserve")
        with mock.patch.object(probe.sys, "platform", "darwin"), mock.patch.dict(os.environ, {probe.RUN_ENV: "1"}), \
                mock.patch.object(probe.official, "copy_private_auth") as auth, \
                mock.patch.object(probe, "Wire") as spawn, self.assertRaises(FileExistsError):
            probe.run(self.root, self.binary, self.auth, probe.CONFIRM)
        auth.assert_not_called()
        spawn.assert_not_called()
        self.assertEqual((self.root / "result.safe.json").read_bytes(), b"preserve")

    def test_bad_native_eof_prevents_second_process_and_still_cleans_auth(self):
        class FailedWire:
            messages = queue.Queue()
            def close(self, graceful):
                return not graceful
        tunnel = mock.Mock(lock=threading.Lock(), forwarded=0, bytes=0, events=[], deadline=time.monotonic() + 10)
        tunnel.start.return_value = 1234
        tunnel.close.return_value = True
        def copy_auth(source, target):
            probe.create_file(target / "auth.json", b"synthetic opaque bytes")
        def phase(ledger, wire, name, prompt, secret):
            ledger.inputs += 1
            return "turn"
        with mock.patch.object(probe.sys, "platform", "darwin"), mock.patch.dict(os.environ, {probe.RUN_ENV: "1"}), \
                mock.patch.object(probe.fixed, "verify_version", return_value=probe.fixed.VERSION_OUTPUTS[probe.VERSION]), \
                mock.patch.object(probe.official, "OfficialTunnel", return_value=tunnel), \
                mock.patch.object(probe, "sandbox_canary"), mock.patch.object(probe.official, "copy_private_auth", side_effect=copy_auth), \
                mock.patch.object(probe.Ledger, "open", return_value=SESSION), \
                mock.patch.object(probe.Ledger, "run_phase", phase), \
                mock.patch.object(probe, "Wire", return_value=FailedWire()) as spawn:
            result = probe.run(self.root, self.binary, self.auth, probe.CONFIRM)
        self.assertFalse(result["passed"])
        self.assertEqual(result["normal_eof_count"], 0)
        self.assertEqual(spawn.call_count, 1)
        self.assertTrue(result["auth_copy_removed"])

    def test_owned_python_process_eof_and_hanging_process_cleanup_are_distinct(self):
        env = {"PATH": "/usr/bin:/bin"}
        wire = probe.Wire([sys.executable, "-B", "-c", "import sys;sys.stdin.buffer.read()"], env, self.base)
        self.assertTrue(wire.close(True))
        wire = probe.Wire([sys.executable, "-B", "-c", "import time;time.sleep(30)"], env, self.base)
        self.assertTrue(wire.close(False))
        self.assertFalse(wire.group_alive())


if __name__ == "__main__":
    unittest.main()
