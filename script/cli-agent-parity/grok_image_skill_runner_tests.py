"""图片技能运行器的离线篡改校验；不作为真实模型验收证据。"""

import base64
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_grok_image_skill_live as runner


class ImageSkillAuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.workdir = Path(self.directory.name).resolve()
        self.evidence = self.workdir / "evidence"
        self.evidence.mkdir()
        self.root, self.fixtures = runner.prepare(self.workdir, self.evidence)
        self.events, self.projections = [], []
        self.history = {"chat_history.jsonl": [], "updates.jsonl": []}
        self.native = "b99a2f00-5cb1-4171-a69b-b54110301f27"
        for generation in ("first-generation", "second-generation"):
            self.events.append({"event": "ready", "generation": generation, "native_session_id": self.native,
                                "version": "1.0.41", "model": "grok-4.7", "policy": "inherit",
                                "resumed": generation == "second-generation"})
        self.events.extend([{"event": "skill_published"}, {"event": "resume_idle_without_replay"}])
        store = self.root / "state/local-cli-attachments"
        store.mkdir()
        for index, (case, keys, colors) in enumerate(runner.CASES):
            # 离线夹具只验证审计关联和字节比较，不模拟图片解码或模型观察。
            data = ("offline-image-" + case).encode()
            digest = runner.previous.sha(data)
            path = store / (digest + ".png")
            path.write_bytes(data)
            content = {"type": "image", "mimeType": "image/png", "data": base64.b64encode(data).decode()}
            content_digest = runner.previous.sha(json.dumps(content, sort_keys=True, separators=(",", ":")).encode())
            generation = "second-generation" if index == 2 else "first-generation"
            message, turn = "message-" + case, "turn-" + case
            identity = {"case": case, "generation": generation, "native_session_id": self.native, "message_id": message}
            text = "按所选顺序执行并识别附图。"
            parts = [{"Text": text}, {"LocalImage": str(path)}]
            parts += [{"Skill": {"name": self.fixtures[key]["name"], "path": self.fixtures[key]["path"]}} for key in keys]
            self.events.extend([
                {"event": "attachment_prepared", "case": case, "mime_type": "image/png", "byte_count": len(data),
                 "sha256": digest, "native_content_digest": content_digest, "source_path": str(path), "persistent_reference_restored": True},
                {"event": "submitted", **identity, "input": {"Submit": {"input": parts}},
                 "markers_absent_from_input": True, "stale_generation_rejected": True},
                {"event": "accepted", **identity, "turn_id": turn},
                {"event": "replay_quiet", "case": case, "message_id": message}])
            answer = "\n".join([self.fixtures[key]["marker"] for key in keys] + [colors])
            self.events.append({"event": "finished", **identity, "turn_id": turn, "output": answer, "outcome": "Completed"})
            wire = (["/" + self.fixtures[keys[0]]["name"] + " " + text] if len(keys) == 1
                    else ["/" + self.fixtures[key]["name"] for key in keys] + [text])
            blocks = [{"type": "text", "text": value} for value in wire] + [content]
            if len(keys) > 1:
                blocks.append({"type": "text", "text": json.dumps({"selected_skills": [
                    {"qualifiedName": "local:" + self.fixtures[key]["name"], "path": self.fixtures[key]["path"]}
                    for key in keys]}, ensure_ascii=False, sort_keys=True, separators=(",", ":"))})
            for item in blocks:
                self.update({"sessionUpdate": "user_message_chunk", "content": item,
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
                info = self.fixtures[key]
                call = "read-" + case + "-" + key
                self.update({"sessionUpdate": "tool_call", "toolCallId": call,
                             "_meta": {"x.ai/tool": {"name": "read_file"}},
                             "rawInput": {"target_file": info["path"]}}, turn)
                self.update({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed",
                             "rawInput": {"variant": "ReadFile", "target_file": info["path"]},
                             "rawOutput": {"FileContent": {"raw_output": (self.evidence / info["fixture"]).read_text()}}}, turn)
            self.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": answer}}, turn)
            self.update({"sessionUpdate": "turn_completed", "prompt_id": turn, "stop_reason": "end_turn"}, turn)
            self.history["chat_history.jsonl"].append({"type": "assistant", "content": answer})
            self.projections.append({"source": "verified_native_final_history", "session_id": self.native, "turn_id": turn,
                                     "runtime_generation": generation, "prompt_index": index, "images": [{"image_sha256": digest,
                                     "image_bytes": len(data), "mime_type": "image/png", "native_content_sha256": content_digest}]})

    def update(self, update, turn):
        self.history["updates.jsonl"].append({"params": {"sessionId": self.native, "update": update,
            "_meta": {"promptId": turn, "eventId": "event-" + str(len(self.history["updates.jsonl"]))}}})

    def audit(self):
        return runner.audit_turns(self.root, self.evidence, self.events, self.fixtures, self.history, self.projections)

    def test_exact_three_turn_history_passes_offline_audit(self):
        self.assertEqual([row["case"] for row in self.audit()], ["initial", "hot", "cold"])

    def test_image_bytes_cannot_be_replaced_in_native_history(self):
        row = next(row for row in self.history["updates.jsonl"] if row["params"]["update"].get("content", {}).get("type") == "image")
        row["params"]["update"]["content"]["data"] = base64.b64encode(b"changed").decode()
        with self.assertRaises(ValueError):
            self.audit()

    def test_native_replay_does_not_count_as_one_input(self):
        self.history["chat_history.jsonl"].append(copy.deepcopy(self.history["chat_history.jsonl"][0]))
        with self.assertRaises(ValueError):
            self.audit()

    def test_output_marker_without_current_skill_read_is_rejected(self):
        self.history["updates.jsonl"] = [row for row in self.history["updates.jsonl"]
            if row["params"]["update"]["sessionUpdate"] not in {"tool_call", "tool_call_update"}]
        with self.assertRaises(ValueError):
            self.audit()

    def test_corrupted_skill_bytes_are_rejected(self):
        row = next(row for row in self.history["updates.jsonl"] if row["params"]["update"]["sessionUpdate"] == "tool_call_update")
        row["params"]["update"]["rawOutput"]["FileContent"]["raw_output"] += "changed"
        with self.assertRaises(ValueError):
            self.audit()

    def test_other_turn_projection_is_rejected(self):
        self.projections[1]["turn_id"] = "turn-initial"
        with self.assertRaises(ValueError):
            self.audit()

    def test_swapped_skill_order_is_rejected(self):
        sent = runner.entries(self.events, "submitted")[1]
        parts = sent["input"]["Submit"]["input"]
        parts[2], parts[3] = parts[3], parts[2]
        with self.assertRaises(ValueError):
            self.audit()

    def test_hidden_marker_in_prompt_is_rejected(self):
        runner.entries(self.events, "submitted")[0]["input"]["Submit"]["input"][0]["Text"] += self.fixtures["alpha"]["marker"]
        with self.assertRaises(ValueError):
            self.audit()

    def test_environment_preserves_default_homes_and_short_tmpdir(self):
        with patch.dict(os.environ, {"HOME": "/Users/example", "CODEX_HOME": "/Users/example/codex",
                                     "TMPDIR": str(self.workdir), "GROK_API_KEY": "not-forwarded"}, clear=True):
            environment = runner.environment_for(self.root, self.workdir, Path("/grok"), Path("/worker"))
        self.assertEqual(environment["HOME"], "/Users/example")
        self.assertEqual(environment["CODEX_HOME"], "/Users/example/codex")
        self.assertEqual(environment["TMPDIR"], str(self.workdir))
        self.assertNotIn("GROK_API_KEY", environment)

    def test_unregistered_tmpdir_environment_is_rejected(self):
        with patch.dict(os.environ, {"HOME": "/Users/example", "TMPDIR": "/private/tmp"}, clear=True):
            with self.assertRaises(ValueError):
                runner.environment_for(self.root, self.workdir, Path("/grok"), Path("/worker"))

    def process_records(self):
        executable = self.workdir / "grok"
        for index, ready in enumerate(runner.entries(self.events, "ready")):
            generation = ready["generation"]
            directory = self.root / "state/cli-agent-processes" / generation
            directory.mkdir(parents=True)
            manifest = {"generation": generation, "executable": str(executable), "cwd": str(self.root / "project"),
                        "arguments": ["--leader", "--leader-socket", str(self.workdir / (generation + ".sock"))]}
            raw_manifest = json.dumps(manifest).encode()
            (directory / "manifest.json").write_bytes(raw_manifest)
            receipt = {"generation": generation, "manifest_sha256": runner.previous.sha(raw_manifest),
                       "cleanup_confirmed": True, "exit_code": 0}
            (directory / "exit.json").write_text(json.dumps(receipt))
            native = json.dumps({"generation": generation, "identity": {"pid": 40000 + index}}).encode()
            (directory / "macos-native.json").write_bytes(native)
            (directory / "macos-cleanup.json").write_text(json.dumps({"generation": generation,
                "native_sha256": runner.previous.sha(native), "job_removed": True,
                "resource_cid_destroyed": True, "execution_failed": False}))
            self.events.append({"event": "cleanup", "generation": generation, "resumed": bool(index),
                                "task_finished_ok": True, "extra_turn_events": 0, "receipt": receipt})
        return executable

    def test_process_audit_preserves_exact_native_cleanup_bytes(self):
        executable = self.process_records()
        with patch.object(runner.os, "kill", side_effect=ProcessLookupError):
            audited = runner.audit_processes(self.root, self.workdir, self.evidence, self.events, executable)
        self.assertEqual(len(audited), 2)
        for row in audited:
            directory = self.root / "state/cli-agent-processes" / row["generation"]
            for name in ("macos-native", "macos-cleanup"):
                self.assertEqual((self.evidence / (row["generation"] + "." + name + ".raw.json")).read_bytes(),
                                 (directory / (name + ".json")).read_bytes())
        self.assertFalse(list(self.evidence.glob("*manifest*")))

    def test_cleanup_from_other_valid_generations_is_rejected(self):
        executable = self.process_records()
        for ready in runner.entries(self.events, "ready"):
            ready["generation"] = "actual-" + ready["generation"]
        with patch.object(runner.os, "kill", side_effect=ProcessLookupError):
            with self.assertRaises(ValueError):
                runner.audit_processes(self.root, self.workdir, self.evidence, self.events, executable)

    def test_cleanup_resume_sequence_must_match_ready_sequence(self):
        executable = self.process_records()
        runner.entries(self.events, "cleanup")[0]["resumed"] = True
        with self.assertRaises(ValueError):
            runner.audit_processes(self.root, self.workdir, self.evidence, self.events, executable)

    def test_ready_resume_sequence_must_be_new_then_resume(self):
        executable = self.process_records()
        runner.entries(self.events, "ready")[1]["resumed"] = False
        with self.assertRaises(ValueError):
            runner.audit_processes(self.root, self.workdir, self.evidence, self.events, executable)


if __name__ == "__main__":
    unittest.main()
