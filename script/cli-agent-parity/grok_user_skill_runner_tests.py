"""用户技能运行器的离线反篡改测试；不替代真实原生验收。"""

import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_grok_user_skill_live as runner


class UserSkillAuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.workdir = Path(self.directory.name).resolve()
        self.evidence = self.workdir / "evidence"
        self.evidence.mkdir()
        self.root, self.fixtures = runner.prepare(self.workdir, self.evidence)
        self.native = "b99a2f00-5cb1-4171-a69b-b54110301f27"
        self.events = [{"event": "ready", "generation": generation, "native_session_id": self.native,
                        "version": "1.0.41", "model": "grok-4.7", "policy": "inherit", "resumed": resumed}
                       for generation, resumed in (("first", False), ("second", True))]
        self.events += [{"event": "skill_published", "path": self.fixtures["gamma"]["path"],
                         "generation": "first", "native_session_id": self.native, "after_case": "initial", "same_connection": True},
                        {"event": "resume_idle_without_replay"}]
        self.traces = []
        for phase, generation, keys in (("ready_catalog", "first", ("alpha", "beta")),
                                       ("reload", "first", ()),
                                       ("refreshed_catalog", "first", ("alpha", "beta", "gamma")),
                                       ("ready_catalog", "second", ("alpha", "beta", "gamma"))):
            commands = [{"name": self.fixtures[key]["name"], "input": None,
                         "_meta": {"bareName": self.fixtures[key]["name"], "scope": self.fixtures[key]["scope"],
                                   "qualifiedName": self.fixtures[key]["qualified_name"], "path": self.fixtures[key]["path"]}}
                        for key in keys]
            self.traces.append({"phase": phase, "generation": generation, "native_session_id": self.native,
                                "proof": {"reloaded": 1} if phase == "reload" else {"commands": commands}})
        self.history = {"chat_history.jsonl": [], "updates.jsonl": []}
        for index, (case, keys) in enumerate(runner.CASES):
            generation, message, turn = "second" if index == 2 else "first", "message-" + case, "turn-" + case
            identity = {"case": case, "generation": generation, "native_session_id": self.native, "message_id": message}
            text = "按本轮所选技能顺序执行。"
            parts = [{"Text": text}] + [{"Skill": {"name": self.fixtures[key]["name"], "path": self.fixtures[key]["path"]}} for key in keys]
            self.events += [{"event": "submitted", **identity, "input": {"Submit": {"input": parts}},
                             "markers_absent_from_input": True, "stale_generation_rejected": True},
                            {"event": "accepted", **identity, "turn_id": turn},
                            {"event": "replay_quiet", "case": case, "message_id": message}]
            answer = "\n".join(self.fixtures[key]["marker"] for key in keys)
            self.events.append({"event": "finished", **identity, "turn_id": turn, "output": answer, "outcome": "Completed"})
            wire = (["/" + self.fixtures[keys[0]]["name"] + " " + text] if len(keys) == 1
                    else ["/" + self.fixtures[key]["name"] for key in keys] + [text])
            if len(keys) > 1:
                wire.append(json.dumps({"selected_skills": [
                    {"qualifiedName": self.fixtures[key]["qualified_name"], "path": self.fixtures[key]["path"]}
                    for key in keys]}, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
            for content in wire:
                self.update({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": content},
                             "_meta": {"promptIndex": index, "modelId": "grok-4.7"}}, turn)
            first = self.fixtures[keys[0]]
            body = (self.evidence / first["fixture"]).read_text().split("---\n", 2)[2]
            attributes = ' args="' + text + '"' if len(keys) == 1 else ""
            expanded_body = body + ("\n\n**ARGUMENTS:** " + text if len(keys) == 1 else "")
            expanded = '<skill name="' + first["name"] + '"' + attributes + '>\n' + expanded_body + '\n</skill>\n'
            expanded += '<skill name="' + first["name"] + '" path="' + first["path"] + '"/>'
            self.history["chat_history.jsonl"].append({"type": "user", "prompt_index": index,
                                                       "content": [{"type": "text", "text": expanded}]})
            for key in keys[1:]:
                info, call = self.fixtures[key], "read-" + case + "-" + key
                self.update({"sessionUpdate": "tool_call", "toolCallId": call,
                             "_meta": {"x.ai/tool": {"name": "read_file"}}, "rawInput": {"target_file": info["path"]}}, turn)
                self.update({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed",
                             "rawInput": {"variant": "ReadFile", "target_file": info["path"]},
                             "rawOutput": {"FileContent": {"raw_output": (self.evidence / info["fixture"]).read_text()}}}, turn)
            self.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": answer}}, turn)
            self.update({"sessionUpdate": "turn_completed", "prompt_id": turn, "stop_reason": "end_turn"}, turn)
            self.history["chat_history.jsonl"].append({"type": "assistant", "content": answer})

    def update(self, update, turn):
        self.history["updates.jsonl"].append({"params": {"sessionId": self.native, "update": update,
            "_meta": {"promptId": turn, "eventId": "event-" + str(len(self.history["updates.jsonl"]))}}})

    def audit(self):
        return runner.audit_turns(self.root, self.evidence, self.events, self.fixtures, self.history, self.traces)

    def test_exact_local_user_hot_and_cold_history_passes(self):
        self.assertEqual([row["selected_sources"] for row in self.audit()],
                         [["local:isp-g06-alpha", "user:isp-g06-beta"], ["user:isp-g06-gamma"],
                          ["user:isp-g06-gamma", "local:isp-g06-alpha"]])

    def test_user_scope_cannot_be_relabelled_local(self):
        self.traces[0]["proof"]["commands"][1]["_meta"]["scope"] = "local"
        with self.assertRaises(ValueError):
            self.audit()

    def test_wire_references_must_match_selected_catalog_paths_and_order(self):
        content = next(row["params"]["update"]["content"] for row in self.history["updates.jsonl"]
                       if row["params"]["update"].get("content", {}).get("text", "").startswith('{"selected_skills":'))
        original = content["text"]
        for change in ("path", "qualifiedName", "order", "unselected"):
            with self.subTest(change=change):
                reference = json.loads(original)
                selected = reference["selected_skills"]
                if change == "path":
                    selected[1]["path"] = self.fixtures["alpha"]["path"]
                elif change == "qualifiedName":
                    selected[1]["qualifiedName"] = "local:" + self.fixtures["beta"]["name"]
                elif change == "order":
                    selected.reverse()
                else:
                    selected.append({"qualifiedName": self.fixtures["gamma"]["qualified_name"],
                                     "path": self.fixtures["gamma"]["path"]})
                content["text"] = json.dumps(reference, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
                with self.assertRaises(ValueError):
                    self.audit()
        content["text"] = original

    def test_unselected_failed_read_still_rejects_the_turn(self):
        wrong = str(self.root / "project/.grok/skills" / self.fixtures["beta"]["name"] / "SKILL.md")
        call = {"sessionUpdate": "tool_call", "toolCallId": "unselected-read",
                "_meta": {"x.ai/tool": {"name": "read_file"}}, "rawInput": {"target_file": wrong}}
        self.update(call, "turn-initial")
        request = self.history["updates.jsonl"].pop()
        request["params"]["_meta"]["eventId"] = "unselected-read-start"
        self.update({"sessionUpdate": "tool_call_update", "toolCallId": "unselected-read", "status": "failed",
                     "rawOutput": {"FileNotFound": {"path": wrong}}}, "turn-initial")
        result = self.history["updates.jsonl"].pop()
        result["params"]["_meta"]["eventId"] = "unselected-read-failed"
        initial_end = next(index for index, row in enumerate(self.history["updates.jsonl"])
                           if row["params"]["update"]["sessionUpdate"] == "turn_completed")
        self.history["updates.jsonl"][initial_end:initial_end] = [request, result]
        with self.assertRaisesRegex(ValueError, "未选择技能之外的工具"):
            self.audit()

    def test_qualified_name_and_original_path_must_match(self):
        for field, value in (("qualifiedName", "local:isp-g06-beta"), ("path", self.fixtures["alpha"]["path"])):
            with self.subTest(field=field):
                metadata = self.traces[0]["proof"]["commands"][1]["_meta"]
                old = metadata[field]
                metadata[field] = value
                with self.assertRaises(ValueError):
                    self.audit()
                metadata[field] = old

    def test_reload_requires_exact_count_and_connection(self):
        for field, value in (("proof", {"reloaded": 2}), ("generation", "second")):
            with self.subTest(field=field):
                old = self.traces[1][field]
                self.traces[1][field] = value
                with self.assertRaises(ValueError):
                    self.audit()
                self.traces[1][field] = old

    def test_initial_catalog_cannot_contain_unpublished_gamma(self):
        self.traces[0]["proof"]["commands"].append(copy.deepcopy(self.traces[2]["proof"]["commands"][2]))
        with self.assertRaises(ValueError):
            self.audit()

    def test_user_skill_read_bytes_cannot_be_replaced(self):
        row = next(row for row in self.history["updates.jsonl"] if row["params"]["update"]["sessionUpdate"] == "tool_call_update")
        row["params"]["update"]["rawOutput"]["FileContent"]["raw_output"] += "替换正文"
        with self.assertRaises(ValueError):
            self.audit()

    def test_expected_answer_without_current_read_is_rejected(self):
        self.history["updates.jsonl"] = [row for row in self.history["updates.jsonl"]
            if row["params"]["update"]["sessionUpdate"] not in {"tool_call", "tool_call_update"}]
        with self.assertRaises(ValueError):
            self.audit()

    def test_native_replay_is_rejected(self):
        self.history["chat_history.jsonl"].append(copy.deepcopy(self.history["chat_history.jsonl"][0]))
        with self.assertRaises(ValueError):
            self.audit()

    def test_hidden_marker_in_prompt_is_rejected(self):
        runner.entries(self.events, "submitted")[0]["input"]["Submit"]["input"][0]["Text"] += self.fixtures["beta"]["marker"]
        with self.assertRaises(ValueError):
            self.audit()

    def test_only_private_grok_home_is_changed(self):
        with patch.dict(os.environ, {"HOME": "/Users/example", "CODEX_HOME": "/Users/example/codex",
                                     "TMPDIR": str(self.workdir), "GROK_API_KEY": "not-forwarded"}, clear=True):
            environment = runner.environment_for(self.root, self.workdir, Path("/grok"), Path("/worker"))
        self.assertEqual(environment["HOME"], "/Users/example")
        self.assertEqual(environment["CODEX_HOME"], "/Users/example/codex")
        self.assertEqual(environment["GROK_HOME"], str(self.root / "home/.grok"))
        self.assertEqual(environment["TMPDIR"], str(self.workdir))
        self.assertEqual(environment["INFINISHELL_GROK_USER_SKILL_TRACE"], "1")
        self.assertNotIn("INFINISHELL_GROK_MANAGED_IMAGE_TRACE", environment)
        self.assertNotIn("GROK_API_KEY", environment)


if __name__ == "__main__":
    unittest.main()
