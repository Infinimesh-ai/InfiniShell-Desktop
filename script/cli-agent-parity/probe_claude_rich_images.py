#!/usr/bin/env python3
"""固定 Claude 的图片格式、纯图片和单技能原生合同校准；不读取认证材料。"""

import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time
import uuid

from PIL import Image

import prepare_claude_cli
import run_claude_adapter_live as adapter


def digest(data):
    return hashlib.sha256(data).hexdigest()


class NativeSession:
    def __init__(self, executable, root, plugin, output, resume=None, skill_tools=False):
        self.output = output
        self.frames = []
        self.pending = queue.Queue()
        self.session = resume or ""
        self.events = []
        arguments = [str(executable), "--print", "--input-format", "stream-json",
                     "--output-format", "stream-json", "--verbose", "--replay-user-messages",
                     "--permission-prompt-tool", "stdio", "--permission-prompts", "host",
                     "--setting-sources", "", "--strict-mcp-config", "--mcp-config",
                     '{"mcpServers":{}}', "--tools", "Skill" if skill_tools else "", "--plugin-dir", str(plugin),
                     "--system-prompt", "For any picture, identify the left and right colors. "
                     "Respond in the format LEFT=<color> RIGHT=<color>, replacing placeholders with uppercase color names from the picture, unless a selected skill requests a prefix. "
                     + ("Use only the explicitly selected Skill tool when requested." if skill_tools else "Do not use tools.")]
        if resume:
            arguments.append(f"--resume={resume}")
        environment = dict(os.environ)
        environment.update(DISABLE_AUTOUPDATER="1", DISABLE_UPDATES="1",
                           CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1")
        self.process = subprocess.Popen(arguments, cwd=root, env=environment, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        self.stderr = bytearray()
        threading.Thread(target=self.read_stdout, daemon=True).start()
        threading.Thread(target=self.read_stderr, daemon=True).start()
        request_id = str(uuid.uuid4())
        self.send({"type": "control_request", "request_id": request_id,
                   "request": {"subtype": "initialize"}})
        response = self.until(lambda frame: frame.get("response", {}).get("request_id") == request_id)
        commands = response.get("response", {}).get("response", {}).get("commands", [])
        self.registered = any(command.get("name") == "infinishell-local-skills:inspect-picture"
                              for command in commands)

    def read_stdout(self):
        for raw in self.process.stdout:
            try:
                frame = json.loads(raw)
            except json.JSONDecodeError:
                frame = {"unparsed_sha256": digest(raw)}
            self.pending.put((raw, frame))
        self.pending.put((b"", None))

    def read_stderr(self):
        for raw in self.process.stderr:
            self.stderr.extend(raw)

    def send(self, frame):
        raw = (json.dumps(frame, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        self.process.stdin.write(raw)
        self.process.stdin.flush()
        # 仅原生用户输入含有自建夹具；控制帧不持久化认证或初始化元数据。
        if frame.get("type") == "user":
            self.frames.append(("stdin", raw))

    def until(self, predicate, timeout=180):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            raw, frame = self.pending.get(timeout=max(0.01, deadline - time.monotonic()))
            if frame is None:
                raise RuntimeError("原生进程在结果前退出")
            self.events.append(frame)
            kind = frame.get("type")
            # 保留本探针消息的原始回放和结果字节，认证初始化及思考正文不进入收据。
            if kind in {"user", "command_lifecycle", "result"}:
                self.frames.append(("stdout", raw))
            if kind == "assistant" and any(block.get("type") == "tool_use"
                    for block in frame.get("message", {}).get("content", [])):
                self.frames.append(("stdout", raw))
            if kind == "control_request":
                request = frame.get("request", {})
                selected = (request.get("tool_name") == "Skill" and request.get("input", {}).get("skill")
                            == "infinishell-local-skills:inspect-picture")
                response = ({"behavior": "allow", "updatedInput": request["input"]} if selected else
                            {"behavior": "deny", "message": "此图片校准只授权精确所选技能。"})
                self.send({"type": "control_response", "response": {"subtype": "success",
                           "request_id": frame["request_id"], "response": response}})
            if kind == "system" and frame.get("subtype") == "init":
                self.session = frame.get("session_id", self.session)
            if predicate(frame):
                return frame
        raise TimeoutError("原生图片校准超时")

    def submit(self, label, content, expected):
        message_id = str(uuid.uuid4())
        start = len(self.events)
        self.send({"type": "user", "session_id": self.session, "uuid": message_id,
                   "parent_tool_use_id": None, "message": {"role": "user", "content": content}})
        result = self.until(lambda frame: frame.get("type") == "result")
        self.session = result.get("session_id", self.session)
        events = self.events[start:]
        replay = [event for event in events if event.get("type") == "user"
                  and event.get("uuid") == message_id and event.get("isReplay")]
        init = next((event for event in events if event.get("type") == "system"
                     and event.get("subtype") == "init"), {})
        return {"label": label, "message_id": message_id, "native_session_id": self.session,
                "model": init.get("model"), "registered_skill": self.registered,
                "is_error": result.get("is_error"), "result": result.get("result"),
                "expected_result": expected, "correct": result.get("result", "").strip() == expected,
                "replay_count": len(replay), "replay_content": [event.get("message", {}).get("content")
                                                                  for event in replay],
                "input_content": content, "skill_calls": [block.get("input") for event in events
                    if event.get("type") == "assistant" for block in event.get("message", {}).get("content", [])
                    if block.get("type") == "tool_use" and block.get("name") == "Skill"]}

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, 15)
            self.process.wait(timeout=10)
        self.output.mkdir(parents=True, exist_ok=False)
        for index, (direction, raw) in enumerate(self.frames):
            (self.output / f"{index:03d}-{direction}.ndjson").write_bytes(raw)
        return {"exit_code": self.process.returncode, "stderr_bytes": len(self.stderr),
                "stderr_sha256": digest(self.stderr), "raw_fixture_frames": len(self.frames)}


def run_production(args, contract):
    if args.supervisor is None or args.case is None:
        raise ValueError("生产校准需要监督者和明确用例")
    args.output.mkdir(parents=True, exist_ok=False)
    root = Path(tempfile.mkdtemp(prefix="infinishell-claude-rich-product-", dir="/private/tmp"))
    settings = adapter.prepare_project(root)
    environment = adapter.authorized_default_account_environment(root)
    evidence = args.output.resolve() / "events.ndjson"
    environment.update({
        "INFINISHELL_CLAUDE_LIVE_ROOT": str(root),
        "INFINISHELL_CLAUDE_LIVE_AUTH_MODE": "authorized_default_account",
        "INFINISHELL_CLAUDE_LIVE_EXECUTABLE": str(args.executable.resolve()),
        "INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION": "2.1.280",
        "INFINISHELL_CLAUDE_LIVE_ARTIFACT": str(evidence),
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor.resolve()),
        "INFINISHELL_CLAUDE_LIVE_MODEL": args.model,
        "INFINISHELL_CLAUDE_RICH_IMAGE_CASE": args.case,
    })
    name = "ai::cli_agent_runtime::claude::live_tests::managed_image_live_tests::real_claude_managed_rich_image_lifecycle"
    settings_sha = digest(settings.read_bytes())
    binding = source_binding()
    timed_out = False
    try:
        result = subprocess.run([str(args.test_binary.resolve()), name, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                env=environment, capture_output=True, text=True, timeout=720)
    except subprocess.TimeoutExpired as error:
        timed_out = True
        result = subprocess.CompletedProcess([], -1,
            (error.stdout or b"").decode("utf-8", "replace"),
            (error.stderr or b"").decode("utf-8", "replace"))
    output = adapter.sanitize(result.stdout + result.stderr, root, None, None)
    (args.output / "test-output.txt").write_text(output)
    events = [json.loads(line) for line in evidence.read_text().splitlines()] if evidence.exists() else []
    trace = [json.loads(line.split("CLAUDE_NATIVE_PROTOCOL_IDS ", 1)[1]) for line in output.splitlines()
             if "CLAUDE_NATIVE_PROTOCOL_IDS " in line]
    prepared = [event for event in events if event.get("event") == "rich_attachment_prepared"]
    replays = [event.get("rich_image_content_projection") for event in trace
               if event.get("rich_image_content_projection") is not None]
    exact_replay = len(prepared) == len(replays) == 1 and (
        prepared[0]["native_array_sha256"] == replays[0]["array_sha256"] and
        replays[0]["images"] == [{"media_type": prepared[0]["media_type"],
                                 "image_sha256": prepared[0]["image_sha256"],
                                 "image_bytes": prepared[0]["image_bytes"]}])
    sources_unchanged = binding["critical_source_sha256"] == source_binding()["critical_source_sha256"]
    passed = (not timed_out and sources_unchanged and result.returncode == 0 and "1 passed" in output and bool(events) and events[-1].get("event") == "acceptance_passed"
              and exact_replay and digest(settings.read_bytes()) == settings_sha)
    receipt = {"scope": "production_adapter_process_resume", "case": args.case, "binary": contract,
               "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
               "test_binary_sha256": digest(args.test_binary.read_bytes()),
               "supervisor_sha256": digest(args.supervisor.read_bytes()), "model": args.model,
               "exit_code": result.returncode, "native_image_replay_matches_prepared_bytes": exact_replay,
               "timed_out": timed_out, "critical_sources_unchanged_during_run": sources_unchanged,
               "acceptance_passed": passed, "credential_material_read": False,
               "app_restart_or_gui_verified": False, "workspace": str(root)}
    receipt.update(binding)
    (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "passed": passed, "case": args.case}))
    if not passed:
        raise SystemExit(1)


def source_binding():
    repository = Path(__file__).resolve().parents[2]
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=repository, text=True)
    files = ['app/src/ai/cli_agent_runtime/claude.rs', 'app/src/ai/cli_agent_runtime/managed_input.rs', 'app/src/ai/cli_agent_runtime/claude_managed_image_live_tests.rs']
    return {"source_commit_is_baseline_only": bool(status.strip()),
            "source_worktree_dirty": bool(status.strip()), "source_status_porcelain": status,
            "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "critical_source_sha256": {name: hashlib.sha256((repository / name).read_bytes()).hexdigest()
                                       for name in files}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--skill-only", action="store_true")
    parser.add_argument("--explicit-skill-tool", action="store_true")
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--supervisor", type=Path)
    parser.add_argument("--case", choices=("jpeg", "webp", "gif", "pure-png"))
    parser.add_argument("--model", default="claude-opus-5-5")
    args = parser.parse_args()
    skill_prompt = ("Invoke the Skill tool with skill=infinishell-local-skills:inspect-picture, then apply that skill to the attached image."
                    if args.explicit_skill_tool else "/infinishell-local-skills:inspect-picture")
    contract = prepare_claude_cli.verify_binary(args.executable, prepare_claude_cli.current_platform(), "2.1.280")
    version = subprocess.check_output([str(args.executable), "--version"], text=True).strip()
    status = json.loads(subprocess.check_output([str(args.executable), "auth", "status"], text=True))
    if version != "2.1.280 (Claude Code)" or status.get("loggedIn") is not True:
        raise ValueError("需要固定已认证 Claude 2.1.280")
    if args.test_binary is not None:
        return run_production(args, contract)
    args.output.mkdir(parents=True, exist_ok=False)
    report = {"scope": "native_contract_only", "version": version, "binary": contract,
              "logged_in": True, "credential_material_read": False, "production_adapter_verified": False,
              "runs": [], "cleanup": [], "source_commit": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], text=True).strip()}
    report.update(source_binding())
    with tempfile.TemporaryDirectory(prefix="infinishell-claude-rich-images-", dir="/private/tmp") as temporary:
        root = Path(temporary)
        plugin = root / "plugin"
        (plugin / ".claude-plugin").mkdir(parents=True)
        (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "infinishell-local-skills", "version": "0.1.0"}))
        skill = plugin / "skills/inspect-picture"
        skill.mkdir(parents=True)
        (skill / "SKILL.md").write_text("---\nname: inspect-picture\ndescription: Inspect the attached picture.\n---\n"
                                       "Inspect the attached picture and prefix the color answer with SKILL_OK. "
                                       "Answer in the format SKILL_OK LEFT=<color> RIGHT=<color>, using actual uppercase color names. Do not use tools.\n")
        picture = Image.new("RGB", (256, 128), "red")
        picture.paste((0, 0, 255), (128, 0, 256, 128))
        blocks = {}
        for name, mime in (("PNG", "image/png"), ("JPEG", "image/jpeg"), ("WEBP", "image/webp"), ("GIF", "image/gif")):
            buffer = io.BytesIO()
            picture.save(buffer, format=name)
            data = buffer.getvalue()
            (args.output / f"fixture.{name.lower()}").write_bytes(data)
            blocks[name] = {"type": "image", "source": {"type": "base64", "media_type": mime,
                                                        "data": base64.b64encode(data).decode()}}
        session = NativeSession(args.executable, root, plugin, args.output / "first-native-frames", skill_tools=args.skill_only)
        try:
            for name, block in ({} if args.skill_only else blocks).items():
                report["runs"].append(session.submit(name, [{"type": "text", "text": "Read this picture."}, block],
                                                       "LEFT=RED RIGHT=BLUE"))
            if not args.skill_only:
                report["runs"].append(session.submit("pure-image", [blocks["PNG"]], "LEFT=RED RIGHT=BLUE"))
            report["runs"].append(session.submit("skill-image", [{"type": "text", "text": skill_prompt},
                                                                   blocks["PNG"]], "SKILL_OK LEFT=RED RIGHT=BLUE"))
        finally:
            report["cleanup"].append(session.close())
            (args.output / "receipt.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        resumed = NativeSession(args.executable, root, plugin, args.output / "resume-native-frames", session.session, skill_tools=args.skill_only)
        try:
            for name, block in ({} if args.skill_only else blocks).items():
                report["runs"].append(resumed.submit("resume-" + name, [{"type": "text", "text": "Read this picture."}, block],
                                                       "LEFT=RED RIGHT=BLUE"))
            if not args.skill_only:
                report["runs"].append(resumed.submit("resume-pure-image", [blocks["PNG"]], "LEFT=RED RIGHT=BLUE"))
            report["runs"].append(resumed.submit("resume-skill-image", [{"type": "text", "text": skill_prompt},
                                                                            blocks["PNG"]], "SKILL_OK LEFT=RED RIGHT=BLUE"))
        finally:
            report["cleanup"].append(resumed.close())
            (args.output / "receipt.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "results": [{key: row[key] for key in
           ("label", "correct", "is_error", "replay_count", "result")} for row in report["runs"]]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
