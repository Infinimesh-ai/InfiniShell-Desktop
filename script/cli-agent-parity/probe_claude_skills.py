#!/usr/bin/env python3
"""只在临时插件中验证 Claude 原生技能注册，不请求模型执行技能。"""

import argparse
import json
from pathlib import Path
import tempfile
import uuid

from probe_local_tools import environment
from probe_protocol import Recorder


class SkillRecorder(Recorder):
    def clean(self, value):
        if isinstance(value, dict) and isinstance(value.get("commands"), list):
            value = {**value, "commands": [{"name": command.get("name")} for command in value["commands"]]}
        return super().clean(value)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--invoke-command", action="store_true", help="无凭据验证 slash 命令解析，预期模型认证失败")
    parser.add_argument("--command-arguments", default="")
    args = parser.parse_args()
    registered = False
    with tempfile.TemporaryDirectory(prefix="infinishell-skill-probe-") as temporary:
        directory = Path(temporary)
        plugin = directory / "plugin"
        (plugin / ".claude-plugin").mkdir(parents=True)
        (plugin / ".claude-plugin" / "plugin.json").write_text(json.dumps({"name": "infinishell-skill-probe", "version": "0.1.0"}))
        skill = plugin / "skills" / "review-local"
        (skill / "references").mkdir(parents=True)
        (skill / "SKILL.md").write_text("---\nname: review-local\ndescription: Read-only local skill registration probe.\n---\nRead references/context.md and report what it says without executing commands.\n")
        (skill / "references" / "context.md").write_text("PROBE_LOCAL_SKILL_CONTEXT\n")
        command = [args.executable, "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--setting-sources", "", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}", "--plugin-dir", str(plugin)]
        if args.invoke_command:
            command.append("--replay-user-messages")
        with args.output.open("w") as output:
            recorder = SkillRecorder(command, environment(directory), directory, output)
            try:
                recorder.send({"type":"control_request", "request_id":"init-skill-probe", "request":{"subtype":"initialize"}})
                response = recorder.until(lambda item: isinstance(item, dict) and item.get("response", {}).get("request_id") == "init-skill-probe")
                commands = response.get("response", {}).get("response", {}).get("commands", []) if response else []
                registered = any(command.get("name") == "infinishell-skill-probe:review-local" for command in commands)
                recorder.record("assertion", {"isolated_plugin_skill_registered": registered, "relative_resource_present": (skill / "references" / "context.md").is_file(), "skill_invocation_tested": False})
                if args.invoke_command and registered:
                    prompt = "/infinishell-skill-probe:review-local"
                    if args.command_arguments:
                        prompt += " " + args.command_arguments
                    recorder.send({"type":"user", "session_id":"", "uuid":str(uuid.uuid4()), "parent_tool_use_id":None,
                        "message":{"role":"user", "content":prompt}})
                    result = recorder.until(lambda item: isinstance(item, dict) and item.get("type") == "result", 20)
                    recorder.record("assertion", {"command_parser_exercised": True,
                        "native_result_received": result is not None,
                        "skill_execution_verified": False})
            finally:
                recorder.close()
    if not registered:
        raise SystemExit("Claude did not register the isolated plugin skill")


if __name__ == "__main__":
    main()
