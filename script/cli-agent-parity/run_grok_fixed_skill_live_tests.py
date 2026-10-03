"""固定技能运行器审计负例；这些离线事件不构成原生验收证据。"""

import copy
import tempfile
import unittest
from pathlib import Path

import run_grok_fixed_skill_live as runner


class FixedSkillAuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="fixed-skill-audit-")
        self.addCleanup(self.directory.cleanup)
        self.evidence = Path(self.directory.name)
        document = "---\nname: isp-g06-fixed-alpha\n---\n中文与 English 原生正文。\nG06_FIXED_ALPHA_test\n"
        (self.evidence / "alpha.md").write_bytes(document.encode("utf-8"))
        self.body = document.split("---\n", 2)[2].strip()
        self.fixtures = {
            "alpha": {"name": "isp-g06-fixed-alpha", "marker": "G06_FIXED_ALPHA_test", "fixture": "alpha.md",
                      "sha256": runner.shared.sha(document.encode())},
            "beta": {"name": "isp-g06-fixed-beta", "marker": "G06_FIXED_BETA_test"},
        }
        self.events, updates, users = [], [], []
        for generation, resumed in (("first", False), ("second", True)):
            self.events.extend([
                {"event": "ready", "generation": generation, "resumed": resumed, "native_session_id": "session",
                 "profile": {"storageId": "profile"}, "managed_home": "/synthetic/profile",
                 "selected_skill_bytes_verified": True, "child_subset_verified": True},
                {"event": "unselected_rejected", "generation": generation, "resumed": resumed, "message_id": "reject-" + generation},
            ])
        for index, case in enumerate(runner.CASES):
            generation, message, turn, call = ("second" if index == 2 else "first"), f"message-{index}", f"turn-{index}", f"call-{index}"
            self.events.extend([
                {"event": "submitted", "case": case, "generation": generation, "message_id": message},
                {"event": "approval", "case": case, "generation": generation, "exact": True,
                 "decision": "DenyOnce" if index == 0 else "AllowOnce"},
                {"event": "turn_verified", "case": case, "generation": generation, "message_id": message, "turn_id": turn,
                 "call_id": call, "native_session_id": "session", "allow": index != 0,
                 "skill_sha256": self.fixtures["alpha"]["sha256"],
                 "final_sha256": runner.shared.sha(self.fixtures["alpha"]["marker"].encode())},
            ])
            users.append({"type": "user", "prompt_index": index})
            values = [
                {"sessionUpdate": "user_message_chunk", "_meta": {"promptIndex": index, "modelId": "grok-4.7"},
                 "content": {"type": "text", "text": f'本轮 {case}： "user:isp-g06-fixed-alpha"'}},
                {"sessionUpdate": "tool_call", "toolCallId": call, "_meta": {"x.ai/tool": {"name": "skill"}},
                 "rawInput": {"name": "user:isp-g06-fixed-alpha"}},
                {"sessionUpdate": "tool_call_update", "toolCallId": call, "kind": "other", "_meta": {"x.ai/tool": {"name": "skill"}},
                 "rawInput": {"variant": "Dynamic", "name": "user:isp-g06-fixed-alpha"}},
                {"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "failed" if index == 0 else "completed"},
            ]
            if index:
                values[-1]["rawOutput"] = runner.expected_skill_output(
                    self.body, Path("/synthetic/profile") / "grok/skills" / "isp-g06-fixed-alpha", "isp-g06-fixed-alpha")
                values.insert(1, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "调用技能前的说明。"}})
                values.append({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": self.fixtures["alpha"]["marker"]}})
            values.append({"sessionUpdate": "turn_completed", "prompt_id": turn, "stop_reason": "cancelled" if index == 0 else "end_turn"})
            for sequence, value in enumerate(values, index * 10):
                updates.append({"method": "_x.ai/session/update" if value["sessionUpdate"] == "turn_completed" else "session/update",
                    "params": {"sessionId": "session", "_meta": {"promptId": turn, "eventId": f"session-{sequence}",
                    "streamStartMs": index * 100 + int(value["sessionUpdate"] == "agent_message_chunk" and value["content"]["text"] == self.fixtures["alpha"]["marker"]),
                    "updateParams": {"toolCallId": call, "kind": "Other"},
                    "cancellationCategory": "PermissionRejected", "cancellationContext": {"tool_name": "skill"}}, "update": value}})
        self.history = {"chat_history.jsonl": users, "updates.jsonl": updates}

    def audit(self, events=None, history=None):
        return runner.audit_history(events or self.events, history or self.history, self.fixtures, self.evidence)

    def test_audit_requires_each_real_input_generation_and_original_profile(self):
        self.assertEqual(len(self.audit()), 3)
        events = copy.deepcopy(self.events)
        runner.entries(events, "turn_verified")[-1]["generation"] = "first"
        with self.assertRaises(ValueError):
            self.audit(events=events)
        events = copy.deepcopy(self.events)
        runner.entries(events, "ready")[-1]["profile"]["extra_skill"] = "beta"
        with self.assertRaises(ValueError):
            self.audit(events=events)

    def test_final_response_keeps_pretool_text_and_uses_the_last_native_stream(self):
        audited = self.audit()
        proof = audited[1]["final_response_proof"]
        self.assertEqual([stream["text"] for stream in proof["response_streams"]],
                         ["调用技能前的说明。", self.fixtures["alpha"]["marker"]])
        self.assertEqual(proof["final_response"], self.fixtures["alpha"]["marker"])
        self.assertEqual(proof["completion_watermark"], "session-16")

    def test_final_response_rejects_missing_stream_same_stream_and_post_completion_text(self):
        history = copy.deepcopy(self.history)
        rows = history["updates.jsonl"]
        final = next(row for row in rows if row["params"]["_meta"]["promptId"] == "turn-1"
                     and row["params"]["update"].get("content", {}).get("text") == self.fixtures["alpha"]["marker"])
        del final["params"]["_meta"]["streamStartMs"]
        with self.assertRaisesRegex(ValueError, "流身份"):
            self.audit(history=history)
        history = copy.deepcopy(self.history)
        rows = history["updates.jsonl"]
        final = next(row for row in rows if row["params"]["_meta"]["promptId"] == "turn-1"
                     and row["params"]["update"].get("content", {}).get("text") == self.fixtures["alpha"]["marker"])
        # 同一流不能靠取最后分片或最后一行丢弃此前正文。
        final["params"]["_meta"]["streamStartMs"] = 100
        with self.assertRaises(ValueError):
            self.audit(history=history)
        extra = copy.deepcopy(final)
        extra["params"]["_meta"].update(eventId="session-17", streamStartMs=102)
        completion = next(i for i, row in enumerate(rows) if row["params"]["update"].get("prompt_id") == "turn-1")
        rows.insert(completion + 1, extra)
        with self.assertRaisesRegex(ValueError, "完成水位"):
            self.audit(history=history)

    def test_final_response_cannot_hide_extra_text_before_marker(self):
        history = copy.deepcopy(self.history)
        final = next(row for row in history["updates.jsonl"] if row["params"]["update"].get("content", {}).get("text") == self.fixtures["alpha"]["marker"])
        final["params"]["update"]["content"]["text"] = "多余正文\n" + self.fixtures["alpha"]["marker"]
        with self.assertRaises(ValueError):
            self.audit(history=history)

    def test_denied_skill_cannot_have_even_null_native_output(self):
        history = copy.deepcopy(self.history)
        denied = next(row["params"]["update"] for row in history["updates.jsonl"]
                      if row["params"]["update"].get("status") == "failed")
        denied["rawOutput"] = None
        with self.assertRaises(ValueError):
            self.audit(history=history)

    def test_terminal_requires_a_previous_typed_update(self):
        history = copy.deepcopy(self.history)
        del history["updates.jsonl"][2]
        with self.assertRaisesRegex(ValueError, "类型更新"):
            self.audit(history=history)

    def test_typed_update_cannot_belong_to_another_call(self):
        history = copy.deepcopy(self.history)
        history["updates.jsonl"][2]["params"]["update"]["toolCallId"] = "other-call"
        with self.assertRaisesRegex(ValueError, "身份不匹配"):
            self.audit(history=history)

    def test_allowed_skill_requires_body_and_rejects_unselected_native_execution(self):
        history = copy.deepcopy(self.history)
        completed = next(row["params"]["update"] for row in history["updates.jsonl"]
                         if row["params"]["update"].get("status") == "completed")
        completed["rawOutput"] = {"skill_message": self.fixtures["alpha"]["marker"]}
        completed["description"] = self.body
        with self.assertRaises(ValueError):
            self.audit(history=history)
        history = copy.deepcopy(self.history)
        call = next(row["params"]["update"] for row in history["updates.jsonl"]
                    if row["params"]["update"].get("toolCallId") == "call-1")
        call["rawInput"]["name"] = "user:isp-g06-fixed-beta"
        with self.assertRaises(ValueError):
            self.audit(history=history)


if __name__ == "__main__":
    unittest.main()
