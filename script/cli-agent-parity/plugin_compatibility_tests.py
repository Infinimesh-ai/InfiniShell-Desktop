#!/usr/bin/env python3
"""离线验证随附 Claude/Codex 修补；载荷由固定协议契约构造，不冒充真实 CLI 回合。"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ASSETS = Path(__file__).resolve().parents[2] / "app/assets/bundled/cli-agent-plugins"
EMITTER = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/claude-2.2.0-emit-terminal-sequence.sh"


class CompatibilityTests(unittest.TestCase):
    def environment(self):
        environment = os.environ.copy()
        for key in ("GROK_HOOK_EVENT", "GROK_SESSION_ID"):
            environment.pop(key, None)
        environment.update({"WARP_CLI_AGENT_PROTOCOL_VERSION": "1", "WARP_CLIENT_VERSION": "infinishell-test-dev"})
        return environment

    def payload(self, agent, value, event="stop", extras=()):
        script = ASSETS / agent / "scripts/build-payload.sh"
        result = subprocess.run(["bash", "-c", 'source "$1"; build_payload "$2" "$3" "${@:4}"', "probe", str(script), json.dumps(value), event, *extras],
                                env=self.environment(), text=True, encoding="utf-8", capture_output=True, check=True)
        return json.loads(result.stdout), result.stdout

    def stop(self, agent, value, environment=None, directory_name="plugin", through_hook=False):
        with tempfile.TemporaryDirectory(prefix="infinishell-plugin-patch-test-") as temporary:
            root = Path(temporary) / directory_name
            shutil.copytree(ASSETS / agent / "scripts", root / "scripts")
            scripts = root / "scripts"
            # 替身只收集通知 JSON，不执行终端输出或任何模型行为。
            notify = scripts / "warp-notify.sh"
            notify.write_text('#!/bin/bash\nprintf "%s" "$2"\n', encoding="utf-8", newline="\n")
            notify.chmod(0o755)
            if agent == "codex":
                (scripts / "should-use-structured.sh").write_text("should_use_structured() { return 0; }\n", encoding="utf-8", newline="\n")
            transcript = root / "transcript.jsonl"
            transcript.write_text(json.dumps({"type": "user", "message": {"content": "NEXT_PROMPT"}}) + "\n" + json.dumps({"type": "assistant", "message": {"content": [{"type": "text", "text": "NEXT_RESPONSE"}]}}))
            value = {**value, "transcript_path": str(transcript)}
            environment = environment or self.environment()
            command = ["bash", (scripts / "on-stop.sh").as_posix()]
            if through_hook:
                hook = json.loads((ASSETS / agent / "hooks/hooks.json").read_text())["hooks"]["Stop"][0]["hooks"][0]
                environment.update({"CLAUDE_PLUGIN_ROOT": root.as_posix(), "PLUGIN_ROOT": root.as_posix()})
                if agent == "claude":
                    # 回放受测 exec form：只替换参数值，绝不把路径重新交给 shell 解析。
                    command = [hook["command"], *[argument.replace("${CLAUDE_PLUGIN_ROOT}", root.as_posix()) for argument in hook["args"]]]
                else:
                    command = ["bash", "-c", hook["command"]]
            result = subprocess.run(command, input=json.dumps(value), cwd=temporary,
                                    env=environment, text=True, encoding="utf-8", capture_output=True, check=True)
            self.assertFalse((Path(temporary) / "INJECTED").exists())
            return json.loads(result.stdout) if result.stdout else None

    def test_hook_commands_preserve_spaces_chinese_and_literal_shell_characters(self):
        names = ["插件 空 格", "插件 ' $(touch INJECTED) `touch INJECTED`"]
        if os.name == "posix":
            names.append('插件 " $(touch INJECTED) `touch INJECTED`')
        for agent, field in (("claude", "prompt_id"), ("codex", "turn_id")):
            for name in names:
                with self.subTest(agent=agent, path=name):
                    response = self.stop(agent, {field: "round", "last_assistant_message": "中文 response"}, directory_name=name, through_hook=True)
                    self.assertEqual(response["event"], "stop")
                    self.assertEqual(response[field], "round")
                    self.assertEqual(response["response"], "中文 response")

    def test_every_hook_uses_the_reviewed_argument_boundary(self):
        for agent in ("claude", "codex"):
            hooks = json.loads((ASSETS / agent / "hooks/hooks.json").read_text())["hooks"]
            self.assertEqual(len(hooks), 7 if agent == "claude" else 5)
            for event, groups in hooks.items():
                self.assertEqual(len(groups), 1)
                hook = groups[0]["hooks"][0]
                if agent == "claude":
                    self.assertEqual(hook["command"], "bash")
                    self.assertEqual(len(hook["args"]), 1)
                    self.assertRegex(hook["args"][0], r"^\$\{CLAUDE_PLUGIN_ROOT\}/scripts/on-[a-z-]+\.sh$")
                else:
                    self.assertNotIn("args", hook)
                    self.assertRegex(hook["command"], r'^bash "\$PLUGIN_ROOT/scripts/on-[a-z-]+\.sh"$')

    def test_each_cli_preserves_its_real_correlation_field(self):
        for agent, field, other in (("claude", "prompt_id", "turn_id"), ("codex", "turn_id", "prompt_id")):
            with self.subTest(agent=agent):
                payload, _ = self.payload(agent, {"session_id": "session", field: "original", other: "unverified", "event_id": "unverified", "sequence": 999})
                self.assertEqual(payload[field], "original")
                self.assertNotIn(other, payload)
                self.assertNotIn("sequence", payload)
                self.assertNotIn("event_id", payload)
                self.assertNotIn("terminal_unverified", payload)

    def test_repeated_text_in_different_native_rounds_keeps_distinct_ids(self):
        for agent, field in (("claude", "prompt_id"), ("codex", "turn_id")):
            first, _ = self.payload(agent, {field: "round-a", "prompt": "same"}, "prompt_submit")
            second, _ = self.payload(agent, {field: "round-b", "prompt": "same"}, "prompt_submit")
            old_stop, _ = self.payload(agent, {field: "round-a"}, "stop")
            self.assertEqual(old_stop[field], first[field])
            self.assertNotEqual(old_stop[field], second[field])

    def test_missing_or_invalid_ids_downgrade_terminal_events(self):
        for agent, field in (("claude", "prompt_id"), ("codex", "turn_id")):
            for identifier in (None, "", 12):
                for event in ("stop", "stop_failure"):
                    payload, _ = self.payload(agent, {field: identifier}, event)
                    self.assertEqual(payload["event"], "notification")
                    self.assertNotIn(field, payload)
                    self.assertTrue(payload["terminal_unverified"])

    def test_current_payload_wins_over_a_newer_transcript(self):
        response = self.stop("claude", {"session_id": "session", "prompt_id": "old-round", "last_assistant_message": "ORIGINAL_RESPONSE"})
        self.assertEqual(response["response"], "ORIGINAL_RESPONSE")
        self.assertEqual(response["prompt_id"], "old-round")
        self.assertNotIn("query", response)
        self.assertNotIn("NEXT_", json.dumps(response))

    def test_no_transcript_fallback_when_stop_has_no_final_response(self):
        for agent, field in (("claude", "prompt_id"), ("codex", "turn_id")):
            response = self.stop(agent, {field: "round"})
            self.assertEqual(response["event"], "notification")
            self.assertEqual(response["response"], "")

    def test_active_stop_hook_does_not_report_completion(self):
        for agent, field in (("claude", "prompt_id"), ("codex", "turn_id")):
            self.assertIsNone(self.stop(agent, {field: "round", "last_assistant_message": "text", "stop_hook_active": True}))

    def test_claude_background_work_keeps_stop_as_notification(self):
        response = self.stop("claude", {"prompt_id": "round", "last_assistant_message": "waiting", "background_tasks": [{"id": "child", "status": "running"}]})
        self.assertEqual(response["event"], "notification")

    def test_grok_inheritance_never_reaches_claude_or_legacy_output(self):
        environment = self.environment()
        environment.update({"GROK_HOOK_EVENT": "stop", "GROK_SESSION_ID": "grok-session", "TERM_PROGRAM": "WarpTerminal"})
        environment.pop("WARP_CLI_AGENT_PROTOCOL_VERSION")
        self.assertIsNone(self.stop("claude", {"prompt_id": "round", "last_assistant_message": "text"}, environment))

    def test_control_characters_cannot_escape_json_transport(self):
        text = "中文\nEnglish\x1b]0;injected\x07\x9c"
        payload, raw = self.payload("codex", {"turn_id": "round"}, extras=("--arg", "response", text))
        self.assertEqual(payload["response"], text)
        self.assertNotIn("\x1b", raw)
        self.assertNotIn("\x9c", raw)

    def test_packaged_replacements_match_reviewed_hashes(self):
        for agent in ("claude", "codex"):
            metadata = json.loads((ASSETS / agent / "PATCH_METADATA.json").read_text())
            for name, checksums in metadata["files"].items():
                self.assertEqual(hashlib.sha256((ASSETS / agent / name).read_bytes()).hexdigest(), checksums["replacement_sha256"])

    def test_modern_claude_keeps_raw_osc_for_native_tmux_handling(self):
        self.assertEqual(hashlib.sha256(EMITTER.read_bytes()).hexdigest(), "c461961b191ceb56cbd33f010a89f4c10d8927628c5aca4da3d79f4a881b3b9f")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copytree(ASSETS / "claude/scripts", root / "scripts")
            shutil.copyfile(EMITTER, root / "scripts/emit-terminal-sequence.sh")
            environment = self.environment()
            environment.update(TMUX="isolated-test", CLAUDE_CODE_VERSION="2.1.273")
            body = '{"response":"中文 $(touch INJECTED)"}'
            result = subprocess.run(["bash", (root / "scripts/warp-notify.sh").as_posix(), "warp://cli-agent", body],
                                    env=environment, cwd=root, capture_output=True, text=True, encoding="utf-8", check=True)
            self.assertEqual(json.loads(result.stdout), {"terminalSequence": "\x1b]777;notify;warp://cli-agent;" + body + "\x07"})
            self.assertNotIn("\x1bPtmux", result.stdout)
            self.assertFalse((root / "INJECTED").exists())


if __name__ == "__main__":
    unittest.main()
