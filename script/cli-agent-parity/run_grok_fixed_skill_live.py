#!/usr/bin/env python3
"""固定 Grok 技能 A 的真实拒绝、允许与冷恢复；仅在登记的本机短目录运行。"""

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
from urllib.parse import quote

import run_grok_image_skill_live as image_runner
import run_grok_multi_skill_live as shared
from prepare_grok_cli import current_platform, verify_binary
from probe_claude_rich_images import registered_macos_root
from run_grok_official_adapter_live import copy_private_auth

REPOSITORY = Path(__file__).resolve().parents[2]
SCOPE = "authenticated_fixed_skill_approval_and_cold_restore"
TEST = "ai::cli_agent_runtime::grok::fixed_skill_live_tests::" + SCOPE
CASES = ("deny", "allow", "cold_allow")
SOURCES = tuple("app/src/ai/cli_agent_runtime/" + name for name in (
    "grok.rs", "grok_profile.rs", "grok_profile_skills.rs", "grok_profile_skills_sources.rs",
    "grok_fixed_skill_live_tests.rs", "permissions.rs", "managed_process.rs", "managed_process_macos.rs"))
require = shared.require


def source_hashes():
    return {name: shared.digest(REPOSITORY / name) for name in SOURCES}


def prepare(workdir, evidence):
    root = workdir / "grok-fixed-skill"
    root.mkdir(mode=0o700)
    for relative in ("home/.grok", "project", "skills", "state"):
        (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
    (root / ".infinishell-grok-fixed-skill-probe").write_text("isolated Grok fixed skill verification\n")
    (evidence / "fixtures").mkdir(mode=0o700)
    fixtures = {}
    for key in ("alpha", "beta"):
        name = "isp-g06-fixed-" + key
        marker = "G06_FIXED_" + key.upper() + "_" + secrets.token_hex(24)
        body = ("本技能只用于中文与 English 原生加载验收。读完本轮 skill 工具结果后，严格只输出末尾标记一行。"
                "不要调用任何其他工具，不运行命令，不读取或修改文件，不访问网络。\n" + marker + "\n")
        document = (f"---\nname: {name}\ndescription: 固定权限原生技能验收。\nuser-invocable: true\n---\n" + body).encode()
        path = root / "skills" / name / "SKILL.md"
        path.parent.mkdir(mode=0o700)
        path.write_bytes(document)
        path.chmod(0o400)
        saved = evidence / "fixtures" / (key + ".md")
        saved.write_bytes(document)
        saved.chmod(0o600)
        fixtures[key] = {"name": name, "path": str(path), "marker": marker, "sha256": shared.sha(document),
                         "body_sha256": shared.sha(body.encode()), "fixture": "fixtures/" + key + ".md"}
    config = root / "home/.grok/config.toml"
    config.write_text(shared.CONFIG)
    config.chmod(0o600)
    shared.write_json(evidence / "fixtures.safe.json", fixtures)
    return root, fixtures


def entries(events, name):
    return [row for row in events if row.get("event") == name]


def managed_home(root, ready):
    storage = ready["profile"]["storageId"]
    require(re.fullmatch(r"[0-9a-f-]{36}", storage) is not None, "私有 profile 身份格式异常")
    home = root / "state/grok-managed" / storage
    require(str(home) == ready["managed_home"] and home.resolve(strict=True) == home, "原生私有目录身份改变")
    return home


def capture_native(root, evidence, ready):
    native = ready["native_session_id"]
    require(re.fullmatch(r"[0-9a-f-]{36}", native) is not None, "原生会话身份格式异常")
    directory = managed_home(root, ready) / "grok/sessions" / quote(str(root / "project"), safe="") / native
    captured = {}
    # 只归档本次精确 profile/cwd/session 的业务行；不归档认证、系统提示与思考内容。
    for filename in ("chat_history.jsonl", "updates.jsonl"):
        path = directory / filename
        raw = shared.plain_bytes(path)
        kept, values, indexes = [], [], []
        for number, line in enumerate(raw.splitlines(keepends=True), 1):
            value = json.loads(line)
            if filename == "chat_history.jsonl":
                include = value.get("type") in {"assistant", "tool"} or (value.get("type") == "user" and "prompt_index" in value)
            else:
                params = value.get("params", {})
                include = params.get("sessionId") == native and params.get("update", {}).get("sessionUpdate") in {
                    "user_message_chunk", "agent_message_chunk", "tool_call", "tool_call_update", "turn_completed", "available_commands_update"}
            if include:
                shared.assert_safe(value)
                kept.append(line)
                values.append(value)
                indexes.append({"source_line": number, "bytes": len(line), "sha256": shared.sha(line)})
        (evidence / ("native-" + filename)).write_bytes(b"".join(kept))
        shared.write_json(evidence / (filename + ".selection.safe.json"), {"source_path": str(path),
            "source_bytes": len(raw), "source_sha256": shared.sha(raw), "selected_lines": indexes})
        captured[filename] = values
    return captured


def contains(value, expected):
    if isinstance(value, str):
        return expected in value
    if isinstance(value, dict):
        return any(contains(child, expected) for child in value.values())
    if isinstance(value, list):
        return any(contains(child, expected) for child in value)
    return False


def expected_skill_output(body, directory, name):
    # 对齐公开 1.0.41 的 ToolOutput::Skill 与 OpenCode skill 格式，不能从其它字段拼凑正文证据。
    message = (f'<skill_content name="{name}">\n# Skill: {name}\n\n{body.strip()}\n\n'
               f'Base directory for this skill: file://{directory}\n'
               'Relative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.\n'
               'Note: file list is sampled.\n\n<skill_files>\n\n</skill_files>\n</skill_content>')
    return {"type": "Skill", "success": True, "tool_result": "Loaded skill: " + name,
            "skill_name": name, "skill_message": message, "error": None}


def final_response_proof(rows, native, call):
    # 原生持久化会合并分片；按响应流身份保留工具前说明，只以完成水位前的最后响应流核对答案。
    streams, closed, sequence = [], set(), -1
    completion, tool_completed = None, False
    for row in rows:
        params, update = row["params"], row["params"]["update"]
        if update["sessionUpdate"] == "tool_call_update" and update.get("status") == "completed":
            require(update["toolCallId"] == call, "最终响应之前的工具身份改变")
            tool_completed = True
        if update["sessionUpdate"] not in {"agent_message_chunk", "turn_completed"}:
            continue
        metadata = params.get("_meta", {})
        identity = metadata.get("eventId", "")
        match = re.fullmatch(re.escape(native) + r"-(0|[1-9][0-9]*)", identity)
        require(params["sessionId"] == native and match is not None, "最终响应缺少精确原生事件身份")
        current = int(match.group(1))
        require(current > sequence and completion is None, "最终响应乱序或越过完成水位")
        sequence = current
        if update["sessionUpdate"] == "turn_completed":
            require(row["method"] == "_x.ai/session/update", "最终响应完成方法错误")
            completion = identity
            continue
        require(row["method"] == "session/update" and update["content"].get("type") == "text"
                and isinstance(update["content"].get("text"), str), "最终响应正文类型错误")
        stream = metadata.get("streamStartMs")
        require(type(stream) is int, "最终响应缺少原生流身份")
        if not streams or streams[-1]["stream_start_ms"] != stream:
            require(stream not in closed, "最终响应复用已关闭的原生流")
            if streams:
                closed.add(streams[-1]["stream_start_ms"])
            streams.append({"stream_start_ms": stream, "event_ids": [], "text": "", "after_tool": tool_completed})
        streams[-1]["event_ids"].append(identity)
        streams[-1]["text"] += update["content"]["text"]
    require(completion is not None and streams and streams[-1]["after_tool"] and streams[-1]["text"],
            "工具完成后缺少独立最终响应")
    return {"completion_watermark": completion, "response_streams": streams,
            "final_response": streams[-1]["text"], "stream_start_ms": streams[-1]["stream_start_ms"]}


def audit_history(events, history, fixtures, evidence):
    ready = entries(events, "ready")
    sent, turns, approvals = (entries(events, name) for name in ("submitted", "turn_verified", "approval"))
    require(len(ready) == 2 and ready[0]["resumed"] is False and ready[1]["resumed"] is True
            and ready[0]["generation"] != ready[1]["generation"]
            and ready[0]["native_session_id"] == ready[1]["native_session_id"]
            and ready[0]["profile"] == ready[1]["profile"], "固定 profile 或冷恢复身份错误")
    require(all(row["selected_skill_bytes_verified"] is True and row["child_subset_verified"] is True for row in ready), "技能字节或子集合未确认")
    require([row["case"] for row in sent] == [row["case"] for row in turns] == [row["case"] for row in approvals] == list(CASES), "三次原生输入、审批或终态不完整")
    require(len({row["turn_id"] for row in turns}) == len({row["message_id"] for row in sent}) == 3, "回合或消息身份重复")
    rejected = entries(events, "unselected_rejected")
    require(len(rejected) == 2 and [row["generation"] for row in rejected] == [row["generation"] for row in ready]
            and rejected[0]["resumed"] is False and rejected[1]["resumed"] is True
            and not ({row["message_id"] for row in rejected} & {row["message_id"] for row in sent}), "未选技能拒绝未关联真实两代")
    users = [row for row in history["chat_history.jsonl"] if row.get("type") == "user"]
    require([row.get("prompt_index") for row in users] == [0, 1, 2], "原生持久历史不是精确三次输入")
    require(not contains(history, fixtures["beta"]["marker"]), "未选技能正文进入原生历史")
    document = shared.plain_bytes(evidence / fixtures["alpha"]["fixture"]).decode()
    body = document.split("---\n", 2)[2].strip()
    native = ready[0]["native_session_id"]
    updates = history["updates.jsonl"]
    inputs = [row["params"]["update"] for row in updates if row["params"]["update"]["sessionUpdate"] == "user_message_chunk"]
    require(len(inputs) == 3, "原生用户消息不是三个独立文本输入")
    audited = []
    for index, (send, turn, approval) in enumerate(zip(sent, turns, approvals)):
        allow = index != 0
        generation = ready[int(index == 2)]["generation"]
        require(send["message_id"] == turn["message_id"] and send["generation"] == turn["generation"] == approval["generation"] == generation
                and turn["native_session_id"] == native and turn["allow"] is allow
                and turn["skill_sha256"] == fixtures["alpha"]["sha256"] and approval["exact"] is True
                and approval["decision"] == ("AllowOnce" if allow else "DenyOnce"), "原生审批与输入身份不匹配")
        original = inputs[index]
        require(original.get("_meta", {}).get("promptIndex") == index and original["_meta"]["modelId"] == "grok-4.7"
                and original["content"]["type"] == "text"
                and f"本轮 {CASES[index]}：" in original["content"]["text"]
                and '"user:' + fixtures["alpha"]["name"] + '"' in original["content"]["text"]
                and not contains(original, fixtures["alpha"]["marker"]), "原生模型、选择名称或隐藏正文输入错误")
        rows = [row for row in updates if row["params"].get("_meta", {}).get("promptId") == turn["turn_id"]
                or row["params"]["update"].get("prompt_id") == turn["turn_id"]]
        calls = [row for row in rows if row["params"]["update"]["sessionUpdate"] == "tool_call"]
        require(len(calls) == 1, "原生不是唯一技能工具调用")
        initial, initial_meta = calls[0]["params"]["update"], calls[0]["params"].get("_meta", {}).get("updateParams", {})
        require(initial["toolCallId"] == initial_meta.get("toolCallId") == turn["call_id"]
                and "kind" not in initial and initial_meta.get("kind") == "Other"
                and initial["_meta"]["x.ai/tool"]["name"] == "skill" and "rawOutput" not in initial
                and initial["rawInput"] == {"name": "user:" + fixtures["alpha"]["name"]}, "原生首次技能调用形状错误")
        tools = [row["params"]["update"] for row in rows if row["params"]["update"]["sessionUpdate"] == "tool_call_update"]
        require(tools and all(row["toolCallId"] == turn["call_id"] for row in tools), "工具回执身份不匹配")
        requested, typed, terminal = False, False, False
        for row in rows:
            update = row["params"]["update"]
            if update["sessionUpdate"] == "tool_call":
                require(not requested and not terminal, "工具调用顺序错误")
                requested = True
            if update["sessionUpdate"] != "tool_call_update":
                continue
            require(requested and not terminal, "工具更新在调用前或终态后出现")
            if "rawInput" in update:
                require(not typed and update.get("kind") == "other" and update.get("_meta", {}).get("x.ai/tool", {}).get("name") == "skill"
                        and update["rawInput"] == {"variant": "Dynamic", "name": "user:" + fixtures["alpha"]["name"]}
                        and "status" not in update and "rawOutput" not in update, "原生类型更新重复、参数改变或顺序错误")
                typed = True
            if update.get("status") in {"completed", "failed", "cancelled"}:
                require(typed, "工具终态之前缺少精确原生类型更新")
                terminal = True
        require(typed and terminal, "工具缺少精确类型更新或终态")
        completions = [row for row in rows if row["params"]["update"]["sessionUpdate"] == "turn_completed"]
        require(len(completions) == 1 and completions[0]["params"]["update"]["stop_reason"] == ("end_turn" if allow else "cancelled"), "原生回合终态错误")
        response = None
        if allow:
            response = final_response_proof(rows, native, turn["call_id"])
            completed = [row for row in tools if row.get("status") == "completed"]
            expected = expected_skill_output(body, Path(ready[int(index == 2)]["managed_home"]) / "grok/skills" / fixtures["alpha"]["name"], fixtures["alpha"]["name"])
            require(len(completed) == 1 and completed[0].get("rawOutput") == expected
                    and completed[0]["rawOutput"].get("success") is True
                    and response["final_response"].strip() == fixtures["alpha"]["marker"]
                    and turn["final_sha256"] == shared.sha(fixtures["alpha"]["marker"].encode()), "技能完整正文或实际最终答案不匹配")
        else:
            require(all("rawOutput" not in row and row.get("status") != "completed" for row in tools)
                    and not contains(rows, fixtures["alpha"]["marker"])
                    and completions[0]["params"]["_meta"]["cancellationCategory"] == "PermissionRejected"
                    and completions[0]["params"]["_meta"]["cancellationContext"]["tool_name"] == "skill", "拒绝技能被执行或取消类别错误")
        audited.append({"case": CASES[index], "turn_id": turn["turn_id"], "call_id": turn["call_id"],
                        "generation": generation, "native_history_verified": True, "allowed": allow,
                        "final_response_proof": response})
    return audited


def audit_processes(root, evidence, events, executable):
    ready, cleanup = entries(events, "ready"), entries(events, "cleanup")
    require(len(ready) == len(cleanup) == 2 and [row["generation"] for row in ready] == [row["generation"] for row in cleanup]
            and cleanup[0]["resumed"] is False and cleanup[1]["resumed"] is True, "两代清理未关联真实连接")
    result = []
    for opened, closed in zip(ready, cleanup):
        generation = opened["generation"]
        home = managed_home(root, opened)
        directory = root / "state/cli-agent-processes" / generation
        raw_manifest = shared.plain_bytes(directory / "manifest.json", 1024 * 1024)
        manifest = json.loads(raw_manifest)
        argv = [shared.os_string(value) for value in manifest["arguments"]]
        require(manifest["generation"] == generation and manifest["executable"] == str(executable)
                and manifest["cwd"] == str(home / "startup")
                and argv == ["agent", "--no-leader", "--agent-profile", str(home / "profile.md"), "stdio"], "不是生产固定私有 profile 启动")
        receipt = json.loads(shared.plain_bytes(directory / "exit.json", 1024 * 1024))
        require(closed["receipt"] == receipt and closed["task_finished_ok"] is True and closed["managed_auth_removed"] is True
                and closed["native_session_id"] == opened["native_session_id"] and closed["phase_error"] is None
                and receipt["generation"] == generation and receipt["manifest_sha256"] == shared.sha(raw_manifest)
                and receipt["cleanup_confirmed"] is True and receipt["exit_code"] == 0 and not (home / "grok/auth.json").exists(), "原生自然退出或认证清理未确认")
        native_raw = shared.plain_bytes(directory / "macos-native.json", 1024 * 1024)
        cleanup_raw = shared.plain_bytes(directory / "macos-cleanup.json", 1024 * 1024)
        native, cleaned = json.loads(native_raw), json.loads(cleanup_raw)
        shared.assert_safe(native)
        shared.assert_safe(cleaned)
        require(native["generation"] == cleaned["generation"] == generation and cleaned["native_sha256"] == shared.sha(native_raw)
                and cleaned["job_removed"] is True and cleaned["resource_cid_destroyed"] is True and cleaned["execution_failed"] is False, "原生 Mac 资源域清理错误")
        (evidence / (generation + ".macos-native.raw.json")).write_bytes(native_raw)
        (evidence / (generation + ".macos-cleanup.raw.json")).write_bytes(cleanup_raw)
        shared.write_json(evidence / (generation + ".exit.safe.json"), receipt)
        result.append({"generation": generation, "manifest_sha256": shared.sha(raw_manifest), "receipt": receipt})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("workdir", "executable", "credential-home", "test-binary", "supervisor"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--build-source-binding", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    require(sys.platform == "darwin", "仅用于本机 Mac 验收")
    workdir = registered_macos_root(args.workdir)
    output = args.output or workdir / "evidence"
    require(output.parent == workdir and not output.exists() and not output.is_symlink(), "证据目录必须是本轮根内的新目录")
    output.mkdir(mode=0o700)
    root, fixtures = prepare(workdir, output)
    receipt = {"scope": SCOPE, "passed": False, "workspace": str(root), "requested_policy": "GrokRestrictedSkillsV1",
               "max_native_inputs": 3, "version": shared.VERSION, "model": "grok-4.7", "source_files_sha256": source_hashes(),
               "gui_verified": False, "cross_platform_verified": False, "child_spawn_verified": False,
               "authentication_material_archived": False, "binary_source_binding_requires_build_receipt": True}
    auth, copied_auth_identity, source_auth, events = None, None, None, []
    settings = args.credential_home / "config.toml"
    settings_sha = None
    try:
        executable, test, supervisor = image_runner.copy_inputs(args, workdir)
        receipt.update(test_binary_sha256=shared.digest(test), supervisor_sha256=shared.digest(supervisor),
                       runner_sha256=shared.digest(Path(__file__).resolve()), native=verify_binary(executable, current_platform("1.0.41"), "1.0.41"),
                       baseline_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True).strip(),
                       source_worktree_dirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=REPOSITORY, text=True).strip()))
        build = json.loads(shared.plain_bytes(args.build_source_binding, 2 * 1024 * 1024))
        require(build["baseline_commit"] == receipt["baseline_commit"]
                and all(build["source_files_sha256"].get(name) == digest for name, digest in receipt["source_files_sha256"].items())
                and build["test_binary_sha256"] == shared.digest(args.test_binary)
                and build["supervisor_sha256"] == shared.digest(args.supervisor), "构建声明、当前源码或原二进制摘要不符")
        receipt.update(build_source_binding_sha256=shared.digest(args.build_source_binding),
                       build_manifest_supplied_by_integrator=True, binary_source_binding_requires_build_receipt=False)
        environment = image_runner.environment_for(root, workdir, executable, supervisor)
        environment.pop("INFINISHELL_GROK_MANAGED_IMAGE_TRACE", None)
        version = subprocess.check_output([str(executable), "--version"], cwd=root / "project", env=environment, text=True, timeout=20).strip()
        require(version == shared.VERSION, "固定官方原生版本改变")
        listing = subprocess.check_output([str(test), "--list"], cwd=REPOSITORY, env=environment, text=True, timeout=45)
        require(TEST + ": test" in listing, "缺少固定技能验收入口")
        source_auth = shared.auth_identity(args.credential_home / "auth.json")
        settings_sha = shared.digest(settings) if settings.exists() else None
        auth = root / "home/.grok/auth.json"
        try:
            require(copy_private_auth(args.credential_home, root / "home/.grok") == auth, "认证副本目标改变")
        finally:
            if auth.exists():
                shared.auth_identity(auth)
                copied_auth_identity = auth.lstat()
        log_path = root / "private-test-output.txt"
        with log_path.open("xb") as log:
            log_path.chmod(0o600)
            process = subprocess.Popen([str(test), TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                       cwd=REPOSITORY, env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            receipt["test_pid"] = process.pid
            try:
                process.wait(timeout=900)
            except subprocess.TimeoutExpired:
                receipt["timed_out"] = True
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=15)
                    receipt["test_process_sigkill_requested"] = True
        receipt["test_exit_code"] = process.returncode
        events = [json.loads(line) for line in shared.plain_bytes(root / "events.ndjson").splitlines() if line]
        shared.assert_safe(events)
        text = shared.plain_bytes(log_path).decode("utf-8", "replace")
        require(process.returncode == 0 and "test result: ok. 1 passed; 0 failed; 0 ignored;" in text
                and events[-1].get("event") == "finished" and events[-1].get("passed") is True and events[-1].get("scope") == SCOPE, "真实固定技能入口未通过")
        history = capture_native(root, output, entries(events, "ready")[0])
        shared.write_json(output / "native-audit.safe.json", {"turns": audit_history(events, history, fixtures, output),
                          "processes": audit_processes(root, output, events, executable)})
        require(source_hashes() == receipt["source_files_sha256"] and shared.digest(test) == receipt["test_binary_sha256"]
                and shared.digest(supervisor) == receipt["supervisor_sha256"]
                and verify_binary(executable, current_platform("1.0.41"), "1.0.41") == receipt["native"], "运行中源码或二进制变化")
        require(all(shared.digest(Path(info["path"])) == info["sha256"] for info in fixtures.values()), "夹具来源字节变化")
        receipt.update(passed=True, native_input_count=3, natural_cleanup_generations=2)
    except Exception as error:
        receipt.update(failure_type=type(error).__name__, failure=shared.SECRET_TEXT.sub("[已移除认证材料]", str(error)))
    finally:
        try:
            if auth is not None and (auth.exists() or auth.is_symlink()):
                current = auth.lstat()
                require(copied_auth_identity is not None and not auth.is_symlink() and current.st_uid == os.getuid()
                        and (current.st_dev, current.st_ino) == (copied_auth_identity.st_dev, copied_auth_identity.st_ino), "认证副本身份改变，保留精确清理")
                auth.unlink()
            receipt["private_auth_copy_removed"] = True
            if source_auth is not None:
                unchanged = (shared.auth_identity(args.credential_home / "auth.json") == source_auth
                             and (shared.digest(settings) if settings.exists() else None) == settings_sha)
                receipt["original_auth_and_settings_unchanged"] = unchanged
                receipt["passed"] &= unchanged
        except Exception as error:
            receipt.update(passed=False, private_auth_copy_removed=False, auth_cleanup_failure_type=type(error).__name__)
        try:
            if not events and (root / "events.ndjson").exists():
                events = [json.loads(line) for line in shared.plain_bytes(root / "events.ndjson").splitlines() if line]
            ready = entries(events, "ready")
            if ready and not (output / "native-chat_history.jsonl").exists():
                capture_native(root, output, ready[0])
            if (root / "events.ndjson").exists():
                shared.assert_safe(events)
                (output / "events.raw.ndjson").write_bytes(shared.plain_bytes(root / "events.ndjson"))
            if (root / "private-test-output.txt").exists():
                raw = shared.plain_bytes(root / "private-test-output.txt")
                shared.assert_safe(raw.decode("utf-8", "replace"))
                require(re.search(rb'"(?:access_token|refresh_token|id_token|api_key|authorization|password)"\s*:', raw, re.I) is None, "日志疑似包含认证字段，禁止归档")
                (output / "test-output.raw.log").write_bytes(raw)
        except Exception as error:
            receipt.update(passed=False, partial_capture_failure_type=type(error).__name__)
        shared.write_json(output / "receipt.safe.json", receipt)
        shared.index_output(output)
    print(json.dumps({"scope": SCOPE, "passed": receipt["passed"], "receipt": str(output / "receipt.safe.json")}, ensure_ascii=False))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
