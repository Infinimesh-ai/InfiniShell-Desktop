#!/usr/bin/env python3
"""固定 Grok 生产适配器的多技能、热新增、冷恢复；默认只准备夹具，--run 才会调用模型。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import tempfile
import time
import tomllib
from urllib.parse import quote

TEST = "ai::cli_agent_runtime::grok::multi_skill_live_tests::real_grok_multi_skill_hot_add_and_resume"
VERSION = "grok 1.0.41 (4220f3b224a6)"
CONFIG = '[cli]\nauto_update = false\n[models]\ndefault = "grok-4.7"\n'
SOURCES = ["app/src/ai/cli_agent_runtime/" + name for name in (
    "grok.rs", "grok_skills.rs", "local_skills.rs", "managed_input.rs", "managed_process.rs",
    "managed_process_macos.rs", "grok_multi_skill_live_tests.rs")]
SECRET_KEY = re.compile(r"^(?:access_token|refresh_token|id_token|api_key|authorization|password|device_code|user_code)$", re.I)
SECRET_TEXT = re.compile(r"Bearer\s+[A-Za-z0-9._-]+|sk-[A-Za-z0-9_-]{16,}")


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def assert_safe(value):
    if isinstance(value, dict):
        require(not any(SECRET_KEY.fullmatch(key) for key in value), "证据出现禁止归档的认证字段")
        for child in value.values():
            assert_safe(child)
    elif isinstance(value, list):
        for child in value:
            assert_safe(child)
    elif isinstance(value, str):
        require(SECRET_TEXT.search(value) is None, "证据疑似包含认证材料")


def write_json(path, value):
    assert_safe(value)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    path.chmod(0o600)


def plain_bytes(path, limit=32 * 1024 * 1024):
    require(not path.is_symlink() and path.is_file(), "证据必须是普通文件")
    require(path.stat().st_size <= limit, "证据文件超出有界读取限制")
    return path.read_bytes()


def source_snapshot(repository):
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=repository, text=True)
    return {"baseline_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip(),
        "source_worktree_dirty": bool(status.strip()), "source_commit_is_baseline_only": bool(status.strip()),
        "source_status_porcelain": status, "source_files_sha256": {name: digest(repository / name) for name in SOURCES},
        "runner_sha256": digest(Path(__file__).resolve())}


def fixture_document(name, marker):
    return (f"---\nname: {name}\ndescription: 无副作用的手动技能验收。\nuser-invocable: true\n"
        "disable-model-invocation: true\n---\n"
        "当用户在本轮明确选择本技能时，先核对本回合的原生展开或只读加载，再输出下面的独立标记。\n"
        + marker + "\n本轮选择了其他技能时，只能读取那些技能的 SKILL.md。未在本回合展开的技能必须重新使用 read_file，"
        "不能从历史记忆复述标记。按本轮选择顺序，每个标记一行，不加解释。不要读取其他文件，不执行命令、不修改文件、不访问网络。\n").encode()


def prepare(output):
    output.mkdir(parents=True, exist_ok=False)
    output.chmod(0o700)
    root = Path(tempfile.mkdtemp(prefix="g6s-", dir="/private/tmp" if Path("/private/tmp").is_dir() else None)).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "project/.grok/skills", "unpublished", "tmp", "state", "cache",
                     "npm-cache", "npm-store", "pip-cache", "xdg-config", "xdg-data"):
        (root / relative).mkdir(parents=True, mode=0o700, exist_ok=True)
    (root / ".infinishell-grok-multi-skill-probe").write_text("isolated Grok multi skill verification\n")
    fixtures = {}
    for key in ("alpha", "beta", "gamma"):
        name = "isp-g06-" + key
        target = root / "project/.grok/skills" / name / "SKILL.md"
        current = root / "unpublished" / name / "SKILL.md" if key == "gamma" else target
        current.parent.mkdir(mode=0o700)
        marker = "G06_" + key.upper() + "_" + secrets.token_hex(24)
        data = fixture_document(name, marker)
        current.write_bytes(data)
        current.chmod(0o400)
        evidence = output / "fixtures" / name / "SKILL.md"
        evidence.parent.mkdir(parents=True, mode=0o700)
        evidence.write_bytes(data)
        fixtures[key] = {"name": name, "path": str(target), "initial_path": str(current),
            "marker": marker, "sha256": sha(data), "bytes": len(data), "fixture": str(evidence.relative_to(output))}
    config = root / "home/.grok/config.toml"
    config.write_text(CONFIG)
    config.chmod(0o600)
    # 信任只授予本轮新建并已审核的合成目录；默认工具权限仍由原生 CLI 决定。
    trust = root / "home/.grok/trusted_folders.toml"
    trust.write_text("[folders." + json.dumps(str(root / "project")) + "]\ntrusted = true\ndecided_at = " + str(int(time.time())) + "\n")
    trust.chmod(0o600)
    write_json(output / "fixtures.safe.json", fixtures)
    return root, fixtures


def isolated_environment(root, executable, supervisor):
    allowed = {"PATH", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME", "SHELL",
               "HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY"}
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    environment.update({"HOME": str(root / "home"), "GROK_HOME": str(root / "home/.grok"),
        "XDG_CONFIG_HOME": str(root / "xdg-config"), "XDG_DATA_HOME": str(root / "xdg-data"),
        "XDG_CACHE_HOME": str(root / "cache"), "TMPDIR": str(root / "tmp"), "TMP": str(root / "tmp"), "TEMP": str(root / "tmp"),
        "npm_config_cache": str(root / "npm-cache"), "npm_config_store_dir": str(root / "npm-store"), "PIP_CACHE_DIR": str(root / "pip-cache"),
        "GROK_DISABLE_API_KEY_AUTH": "1", "GROK_AUTO_UPDATE": "0", "GROK_DISABLE_AUTOUPDATER": "1",
        "GROK_CLAUDE_HOOKS_ENABLED": "0", "GROK_CLAUDE_MCPS_ENABLED": "0", "GROK_CODEX_HOOKS_ENABLED": "0", "GROK_CODEX_MCPS_ENABLED": "0",
        "INFINISHELL_GROK_LIVE_ROOT": str(root), "INFINISHELL_GROK_LIVE_EXECUTABLE": str(executable),
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(supervisor)})
    return environment


def capture_native(root, output, native):
    # 仅精确当前 cwd/native id 的两份历史；不 glob 其他会话，不读取认证或完整 manifest。
    require(re.fullmatch(r"[0-9a-f-]{36}", native) is not None, "原生会话 ID 格式异常")
    directory = root / "home/.grok/sessions" / quote(str(root / "project"), safe="") / native
    result = {}
    for filename in ("chat_history.jsonl", "updates.jsonl"):
        path = directory / filename
        data = plain_bytes(path)
        selected, values, index = [], [], []
        for number, line in enumerate(data.splitlines(keepends=True), 1):
            value = json.loads(line)
            if filename == "chat_history.jsonl":
                include = value.get("type") in {"assistant", "tool"} or (value.get("type") == "user" and "prompt_index" in value)
            else:
                params = value.get("params", {})
                include = params.get("sessionId") == native and params.get("update", {}).get("sessionUpdate") in {
                    "user_message_chunk", "agent_message_chunk", "tool_call", "tool_call_update", "turn_completed", "available_commands_update"}
            if include:
                assert_safe(value)
                selected.append(line)
                values.append(value)
                index.append({"source_line": number, "sha256": sha(line), "bytes": len(line)})
        destination = "native-" + filename
        (output / destination).write_bytes(b"".join(selected))
        write_json(output / (filename + ".selection.safe.json"), {"source_path": str(path), "source_sha256": sha(data),
            "source_bytes": len(data), "selected_lines": index, "archive": destination,
            "excluded": "系统、身份、推理及加密历史不归档；入选行保持原始字节"})
        result[filename] = values
    return result


def audit_turns(native, events, fixtures, history, output):
    ready = [event for event in events if event.get("event") == "ready"]
    finished = [event for event in events if event.get("event") == "finished"]
    submitted = [event for event in events if event.get("event") == "submitted"]
    accepted = [event for event in events if event.get("event") == "accepted"]
    require(len(ready) == 2 and ready[0]["generation"] != ready[1]["generation"]
        and all(value["native_session_id"] == native and value["version"] == "1.0.41" and value["model"] == "grok-4.7" and value["policy"] == "inherit" for value in ready), "真实会话、版本或模型不匹配")
    require([value["case"] for value in finished] == ["initial", "hot", "cold"] and len(submitted) == len(accepted) == 3, "输入、ACK 或完成数量异常")
    require(sum(event.get("event") == "skill_published" for event in events) == 1
        and sum(event.get("event") == "resume_idle_without_replay" for event in events) == 1
        and sum(event.get("event") == "replay_quiet" for event in events) == 3, "热新增、冷恢复或去重观察缺失")
    require(len({value["message_id"] for value in submitted}) == len({value["turn_id"] for value in finished}) == 3, "原生输入或消息 ID 重用")
    native_users = [row for row in history["chat_history.jsonl"] if row.get("type") == "user"]
    require([row.get("prompt_index") for row in native_users] == [0, 1, 2], "原生持久历史重复执行或输入缺失")
    updates = history["updates.jsonl"]
    starts = [index for index, row in enumerate(updates) if row["params"]["update"]["sessionUpdate"] == "user_message_chunk"
              and row["params"]["update"].get("_meta", {}).get("promptIndex") !=
              (updates[index - 1]["params"]["update"].get("_meta", {}).get("promptIndex") if index else None)]
    require(len(starts) == 3, "原生用户输入没有三个独立 promptIndex")
    verified = []
    for position, keys in enumerate((("alpha", "beta"), ("gamma",), ("gamma", "beta"))):
        done, sent, ack = finished[position], submitted[position], accepted[position]
        require(done["case"] == sent["case"] == ack["case"] and done["message_id"] == sent["message_id"] == ack["message_id"]
            and done["turn_id"] == ack["turn_id"] and done["native_session_id"] == sent["native_session_id"] == ack["native_session_id"] == native
            and done["generation"] == sent["generation"] == ack["generation"] == ready[int(position == 2)]["generation"], "适配器消息、回合、运行代次关联错误")
        require(sent["stale_generation_rejected"] and sent["markers_absent_from_input"], "旧回调或秘密输入负例未通过")
        require(all(info["marker"] not in json.dumps(sent["input"]) for info in fixtures.values()), "输入泄漏技能秘密标记")
        turn = done["turn_id"]
        rows = updates[starts[position]:starts[position + 1] if position + 1 < len(starts) else len(updates)]
        original = [row["params"]["update"] for row in rows if row["params"]["update"]["sessionUpdate"] == "user_message_chunk"]
        require(all(row.get("_meta", {}).get("promptIndex") == position and row["_meta"]["modelId"] == "grok-4.7" for row in original), "原生输入的模型或次序错误")
        wire_text = [row["content"]["text"] for row in original]
        text = sent["input"]["Submit"]["input"][0]["Text"]
        expected_wire = (["/" + fixtures[keys[0]]["name"] + " " + text] if len(keys) == 1
            else ["/" + fixtures[key]["name"] for key in keys] + [text])
        require(wire_text == expected_wire, "原生 typed 技能或文字与实际提交不一致")
        calls, expanded = {}, {}
        native_text = "\n".join(item.get("text", "") for item in native_users[position]["content"])
        for key in keys:
            info = fixtures[key]
            body = plain_bytes(output / info["fixture"]).decode().split("---\n", 2)[2]
            # 单技能带指令时，原生展开会附加精确 args 与 ARGUMENTS；正文仍须逐字节匹配。
            arguments = text if len(keys) == 1 else None
            attributes = ' args="' + arguments + '"' if arguments is not None else ""
            match = re.search('<skill name="' + re.escape(info["name"]) + '"' + re.escape(attributes) + '>\n(.*?)\n</skill>', native_text, re.S)
            if match:
                expanded_body = body + ("\n\n**ARGUMENTS:** " + arguments if arguments is not None else "")
                require(match.group(1).encode() == expanded_body.encode(), "原生展开正文不是技能源字节")
                require('<skill name="' + info["name"] + '" path="' + info["path"] + '"/>' in native_text, "原生展开路径错误")
                expanded[key] = {"body_sha256": sha(body.encode()), "body_bytes": len(body.encode()), "path_exact": True}
        terminals, answers = [], []
        for row in rows:
            params, update = row["params"], row["params"]["update"]
            kind = update["sessionUpdate"]
            require(params["sessionId"] == native, "原生历史跨会话")
            if kind in {"tool_call", "tool_call_update", "agent_message_chunk"}:
                require(params.get("_meta", {}).get("promptId") == turn, "原生工具或输出跨回合")
            if kind == "tool_call":
                key = next((key for key in keys if update.get("rawInput", {}).get("target_file") == fixtures[key]["path"]), None)
                require(key is not None and update.get("_meta", {}).get("x.ai/tool", {}).get("name") == "read_file", "发生了所选技能外的工具")
                ident = update["toolCallId"]
                require(ident not in calls, "原生工具 ID 重用")
                calls[ident] = {"skill": key, "typed_input": False, "completed": False, "creation_event_id": params["_meta"]["eventId"]}
            if kind in {"tool_call", "tool_call_update"}:
                require(update["toolCallId"] in calls, "工具更新先于工具身份")
                call = calls[update["toolCallId"]]
                if kind == "tool_call_update" and "rawInput" in update:
                    require(update["rawInput"] == {"variant": "ReadFile", "target_file": fixtures[call["skill"]]["path"]}, "原生工具最终参数非精确只读路径")
                    call["typed_input"] = True
                if update.get("status") == "completed":
                    info = fixtures[call["skill"]]
                    data = update["rawOutput"]["FileContent"]["raw_output"].encode()
                    require(data == plain_bytes(output / info["fixture"]), "read_file 实际字节与技能源不符")
                    call.update(completed=True, sha256=sha(data), bytes=len(data), completion_event_id=params["_meta"]["eventId"])
            if kind == "turn_completed":
                require(update["prompt_id"] == turn and update["stop_reason"] == "end_turn", "原生终态关联错误")
                terminals.append(params["_meta"]["eventId"])
            if kind == "agent_message_chunk":
                answers.append(update["content"]["text"])
        require(len(terminals) == 1 and all(call["typed_input"] and call["completed"] for call in calls.values()), "终态缺失、重复或工具未闭合")
        require(all(key in expanded or any(call["skill"] == key for call in calls.values()) for key in keys), "仅提及技能名或复述历史，未证明本回合实际加载")
        expected = "\n".join(fixtures[key]["marker"] for key in keys)
        # 适配器保留工具前的说明；完整流与最终答案分别核验，不能把说明当成最终答案。
        chat = history["chat_history.jsonl"]
        user_positions = [index for index, row in enumerate(chat) if row.get("type") == "user"]
        turn_chat = chat[user_positions[position]:user_positions[position + 1] if position + 1 < len(user_positions) else len(chat)]
        assistant = [row for row in turn_chat if row.get("type") == "assistant"]
        require(answers and "".join(answers).strip() == done["output"].strip()
            and assistant and not assistant[-1].get("tool_calls")
            and assistant[-1].get("content", "").strip() == expected
            and done["outcome"] == "Completed", "完整输出流或原生最终答案不符")
        verified.append({"case": done["case"], "runtime_generation": done["generation"], "message_id": done["message_id"],
            "native_session_id": native, "prompt_id": turn, "prompt_index": position, "expanded": expanded,
            "read_file": calls, "terminal_event_id": terminals[0], "approval_count": done["approval_count"],
            "markers_exact": True, "native_input_exact": True})
    return verified


def os_string(value):
    if isinstance(value, str):
        return value
    require(isinstance(value, dict) and set(value) == {"Unix"}, "无法审计此平台的原生参数表示")
    return bytes(value["Unix"]).decode("utf-8")


def audit_processes(root, output, events, executable):
    cleaned = [event for event in events if event.get("event") == "cleanup"]
    require(len(cleaned) == 2, "没有两次独立监督者清理收据")
    rows, sockets = [], set()
    for event in cleaned:
        generation = event["generation"]
        directory = root / "state/cli-agent-processes" / generation
        # manifest 含私有控制 token，只在内存核验指定白名单，不归档完整内容。
        manifest_bytes = plain_bytes(directory / "manifest.json", 1024 * 1024)
        manifest = json.loads(manifest_bytes)
        arguments = [os_string(value) for value in manifest["arguments"]]
        require(manifest["generation"] == generation and manifest["executable"] == str(executable)
            and manifest["cwd"] == str(root / "project") and arguments.count("--leader") == 1
            and "--agent-profile" not in arguments and "--no-leader" not in arguments
            and arguments.count("--leader-socket") == 1, "未使用默认生产独占 leader")
        socket = Path(arguments[arguments.index("--leader-socket") + 1])
        require(socket.is_absolute() and socket.is_relative_to(root / "tmp") and socket not in sockets, "leader socket 不独立或逃离私有临时目录")
        sockets.add(socket)
        receipt = event["receipt"]
        require(event["task_finished_ok"] and event["extra_turn_events"] == 0 and receipt is not None
            and receipt["generation"] == generation and receipt["manifest_sha256"] == sha(manifest_bytes)
            and receipt["cleanup_confirmed"] and receipt["exit_code"] == 0, "不能以强杀或无来源回执冒充自然清理")
        row = {"generation": generation, "arguments": arguments, "executable": manifest["executable"],
            "cwd": manifest["cwd"], "manifest_sha256": sha(manifest_bytes), "receipt": receipt,
            "private_unique_leader_socket": str(socket), "socket_absent_after_cleanup": not socket.exists()}
        require(not socket.exists(), "本轮 leader socket 仍存在")
        # 原生 PID 证明仅用于 macOS 独立观察；跨平台完成状态仍由生产 confirmed_exit 验证。
        native_file = directory / "macos-native.json"
        if native_file.exists():
            data = plain_bytes(native_file, 1024 * 1024)
            value = json.loads(data)
            assert_safe(value)
            destination = output / (generation + ".macos-native.raw.json")
            destination.write_bytes(data)
            row["macos_native_record_sha256"] = sha(data)
            require(value["generation"] == generation, "原生 PID 记录跨代次")
            pid = value["identity"]["pid"]
            require(type(pid) is int and pid > 1, "原生 PID 不合法")
            try:
                os.kill(pid, 0)
                absent = False
            except ProcessLookupError:
                absent = True
            except PermissionError:
                absent = False
            row["native_pid"] = pid
            row["native_pid_absent_observed"] = absent
            require(absent, "原生 PID 仍存在或身份无法独立确认")
            cleanup_path = directory / "macos-cleanup.json"
            cleanup_bytes = plain_bytes(cleanup_path, 1024 * 1024)
            proof = json.loads(cleanup_bytes)
            assert_safe(proof)
            require(proof["generation"] == generation and proof["native_sha256"] == sha(data)
                and proof["job_removed"] and proof["resource_cid_destroyed"] and not proof["execution_failed"], "macOS 资源域清理证明不匹配")
            (output / (generation + ".macos-cleanup.raw.json")).write_bytes(cleanup_bytes)
            row["macos_cleanup_proof_sha256"] = sha(cleanup_bytes)
        rows.append(row)
    return rows


def auth_identity(path):
    stat = path.lstat()
    require(not path.is_symlink() and path.is_file() and stat.st_uid == os.getuid()
        and stat.st_nlink == 1 and stat.st_mode & 0o077 == 0, "认证源文件身份或权限不安全")
    return [stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns, stat.st_mode]


def index_output(output):
    rows = []
    for path in sorted(output.rglob("*")):
        if path.is_file() and path.name != "index.safe.json":
            rows.append({"path": str(path.relative_to(output)), "sha256": digest(path), "bytes": path.stat().st_size})
    write_json(output / "index.safe.json", {"files": rows, "authentication_material_archived": False})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="明确启用三次在线模型输入")
    parser.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--credential-home", type=Path)
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--supervisor", type=Path)
    parser.add_argument("--build-source-binding", type=Path,
        help="整合者生成 JSON：baseline_commit/source_files_sha256/test_binary_sha256/supervisor_sha256；运行器核当前字节")
    args = parser.parse_args()
    require(os.name == "posix", "此认证副本运行器仅支持 macOS/Linux；Windows 认证和 ACL 另验")
    repository = args.repository.resolve(strict=True)
    output = args.output.resolve()
    root, fixtures = prepare(output)
    receipt = {"scope": "production_adapter_grok_multi_skill_hot_add_cold_resume", "workspace": str(root),
        "platform": sys.platform, "version": "1.0.41", "model": "grok-4.7", "max_model_inputs": 3,
        "passed": False, "prepared_only": not args.run, "coordinator_persistence_verified": False,
        "gui_or_app_restart_verified": False, "full_lifecycle_or_cross_platform_verified": False,
        "requested_policy": "inherit", "parent_permission_ceiling": None, "profile_override": False,
        "permission_rules_override": False, "approval_scope": "只允许本轮所选技能的精确 read_file；默认只读若无审批则如实记零。",
        "private_project_trust_only": True, "authentication_material_archived": False,
        "raw_reload_catalog_rpc_captured": False,
        "reload_scope": "生产源码的精确 reload→catalog 门禁及同会话原生正文/路径联合证明；不旁路发送管理 RPC。"}
    if not args.run:
        write_json(output / "receipt.json", receipt)
        index_output(output)
        print(json.dumps({"prepared_only": True, "workspace": str(root), "output": str(output), "model_inputs": 0}))
        return
    require(all((args.executable, args.credential_home, args.test_binary, args.supervisor, args.build_source_binding)), "在线运行缺少固定二进制、认证目录或构建来源绑定")
    # 复用图片运行器的官方认证复制流程；本脚本不实现第二套认证读取器。
    sys.path.insert(0, str(repository / "script/cli-agent-parity"))
    from prepare_grok_cli import current_platform, verify_binary
    from run_grok_official_adapter_live import copy_private_auth, audit_private_settings
    from run_claude_adapter_live import sanitize
    auth = None
    events = []
    process = None
    try:
        executable = args.executable.resolve(strict=True)
        test_binary = args.test_binary.resolve(strict=True)
        supervisor = args.supervisor.resolve(strict=True)
        if sys.platform == "darwin":
            require(all(not str(path).startswith("/Volumes/") for path in (executable, test_binary, supervisor)), "运行二进制需先复制到内置盘并核摘要")
        source = source_snapshot(repository)
        build = json.loads(plain_bytes(args.build_source_binding, 2 * 1024 * 1024))
        require(build["baseline_commit"] == source["baseline_commit"]
            and all(build["source_files_sha256"].get(name) == value for name, value in source["source_files_sha256"].items())
            and build["test_binary_sha256"] == digest(test_binary) and build["supervisor_sha256"] == digest(supervisor), "构建声明、当前关键源码或二进制摘要不匹配")
        receipt.update(source)
        receipt.update(test_binary_sha256=digest(test_binary), supervisor_sha256=digest(supervisor),
            build_source_binding_sha256=digest(args.build_source_binding), build_manifest_supplied_by_integrator=True)
        native = verify_binary(executable, current_platform("1.0.41"), "1.0.41")
        receipt["native"] = native
        environment = isolated_environment(root, executable, supervisor)
        # 版本与 ignored 测试列表校验均在复制认证前完成，不会请求模型。
        version = subprocess.check_output([str(executable), "--version"], env=environment, cwd=root / "project", text=True, timeout=15).strip()
        require(version == VERSION, "实际原生 CLI 版本不匹配")
        listing = subprocess.check_output([str(test_binary), "--list"], env=environment, cwd=repository, text=True, timeout=30)
        require(TEST + ": test" in listing, "测试二进制未包含本次生产技能用例")
        config_before = plain_bytes(root / "home/.grok/config.toml")
        original_settings = args.credential_home / "config.toml"
        original_digest = digest(original_settings) if original_settings.exists() else None
        original_auth = auth_identity(args.credential_home / "auth.json")
        auth = root / "home/.grok/auth.json"
        require(copy_private_auth(args.credential_home, root / "home/.grok") == auth, "认证副本路径意外改变")
        receipt["auth_copy_created"] = True
        # 原始测试输出留在私有目录；公开证据只写既有 sanitizer 的输出。
        raw_log = root / "private-test-output.txt"
        with raw_log.open("xb") as log:
            raw_log.chmod(0o600)
            process = subprocess.Popen([str(test_binary), TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                env=environment, cwd=repository, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                process.wait(timeout=720)
            except subprocess.TimeoutExpired:
                receipt["timed_out"] = True
                # 只结束本运行器直接创建的测试进程；监督者应按失联合同独立回收。
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=15)
                    receipt["test_process_sigkill_requested"] = True
        receipt["test_exit_code"] = process.returncode
        text = sanitize(plain_bytes(raw_log).decode("utf-8", "replace"), root, None, None)
        require(SECRET_TEXT.search(text) is None, "清理后的输出仍疑似包含认证材料")
        (output / "test-output.txt").write_text(text)
        raw_events = plain_bytes(root / "events.ndjson")
        events = [json.loads(line) for line in raw_events.splitlines() if line]
        assert_safe(events)
        (output / "events.raw.ndjson").write_bytes(raw_events)
        ready = [event for event in events if event.get("event") == "ready"]
        # 即使首轮失败也保留已取得的精确会话证据，不能先 assert 成功再丢弃负例。
        if ready:
            native_history = capture_native(root, output, ready[0]["native_session_id"])
        require(process.returncode == 0 and events[-1].get("event") == "adapter_flow_passed", "生产适配器测试失败，保留原始事件和私有错误")
        turns = audit_turns(ready[0]["native_session_id"], events, fixtures, native_history, output)
        processes = audit_processes(root, output, events, executable)
        write_json(output / "native-audit.safe.json", {"verified": True, "turns": turns, "processes": processes,
            "coordinator_persistence_verified": False, "input_count": 3, "duplicate_messages_created_no_extra_native_turn": True})
        receipt["passed"] = True
        receipt["authenticated_online_model_verified"] = True
    except Exception as error:
        # 不向聊天输出异常中的任意 CLI 内容；错误仅写经清理的本次私有证据。
        receipt["failure_type"] = type(error).__name__
        receipt["failure"] = SECRET_TEXT.sub("[已移除认证材料]", str(error))
    finally:
        finalization_failures = []
        if auth is not None:
            try:
                auth.unlink(missing_ok=True)
                receipt["private_auth_copy_removed"] = not auth.exists()
            except OSError:
                receipt["private_auth_copy_removed"] = False
                finalization_failures.append("私有认证副本删除失败")
            try:
                receipt["original_auth_stat_unchanged"] = auth_identity(args.credential_home / "auth.json") == original_auth
                receipt["original_settings_unchanged"] = (digest(original_settings) if original_settings.exists() else None) == original_digest
                receipt["private_settings_audit"] = audit_private_settings(config_before, plain_bytes(root / "home/.grok/config.toml"))
                receipt["fixture_bytes_unchanged"] = all(digest(Path(info["path"]) if Path(info["path"]).exists() else Path(info["initial_path"])) == info["sha256"] for info in fixtures.values())
                receipt["source_and_binaries_unchanged"] = (source_snapshot(repository)["source_files_sha256"] == source["source_files_sha256"]
                    and digest(test_binary) == receipt["test_binary_sha256"] and digest(supervisor) == receipt["supervisor_sha256"]
                    and verify_binary(executable, current_platform("1.0.41"), "1.0.41") == native)
                receipt["passed"] = receipt["passed"] and all(receipt.get(key) is True for key in (
                    "private_auth_copy_removed", "original_auth_stat_unchanged", "original_settings_unchanged", "fixture_bytes_unchanged", "source_and_binaries_unchanged")) and receipt["private_settings_audit"]["settings_scope_verified"]
            except Exception:
                finalization_failures.append("运行后认证身份、源文件或二进制复核失败")
        # 首次握手或模型失败也保留已产生的安全原始事件；不能只封存成功路径。
        try:
            events_path = root / "events.ndjson"
            if events_path.exists() and not (output / "events.raw.ndjson").exists():
                raw_events = plain_bytes(events_path)
                events = [json.loads(line) for line in raw_events.splitlines() if line]
                assert_safe(events)
                (output / "events.raw.ndjson").write_bytes(raw_events)
            ready = [event for event in events if event.get("event") == "ready"]
            if ready and not (output / "native-chat_history.jsonl").exists():
                capture_native(root, output, ready[0]["native_session_id"])
        except Exception:
            finalization_failures.append("部分原生证据尚不可安全归档；原件保留在私有目录")
        receipt["runtime_cleanup_observed_count"] = sum(event.get("event") == "cleanup" and bool(event.get("receipt", {}).get("cleanup_confirmed")) for event in events if event.get("receipt") is not None)
        if process is not None:
            # 失败/超时收尾单独计证，不把之后监督者强制回收冒充自然退出成功。
            deadline = time.monotonic() + 45
            generations = [event["generation"] for event in events if event.get("event") == "connecting"]
            while generations and time.monotonic() < deadline:
                if all((root / "state/cli-agent-processes" / generation / "exit.json").exists() for generation in generations):
                    break
                time.sleep(0.25)
            receipt["final_exit_observations"] = []
            for generation in generations:
                path = root / "state/cli-agent-processes" / generation / "exit.json"
                try:
                    data = plain_bytes(path, 1024 * 1024)
                    value = json.loads(data)
                    assert_safe(value)
                    require(value.get("generation") == generation, "退出收据代次错误")
                    (output / (generation + ".exit.raw.json")).write_bytes(data)
                    receipt["final_exit_observations"].append({"generation": generation, "receipt": value, "sha256": sha(data),
                        "production_confirmed_exit_seen": any(event.get("event") == "cleanup" and event.get("generation") == generation and event.get("receipt") == value for event in events)})
                except Exception:
                    finalization_failures.append("监督者退出缺少可核对收据，需按代次独立收尾")
        receipt["finalization_failures"] = finalization_failures
        receipt["passed"] = receipt["passed"] and not finalization_failures
        write_json(output / "receipt.json", receipt)
        index_output(output)
    print(json.dumps({"output": str(output), "passed": receipt["passed"], "scope": receipt["scope"]}))
    if not receipt["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
