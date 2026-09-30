#!/usr/bin/env python3
"""固定 Grok 的图片、技能热新增与冷恢复真实验收；仅使用已登记的本机短目录。"""

import argparse
import base64
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import time

import run_grok_multi_skill_live as previous
from prepare_grok_cli import current_platform, verify_binary
from probe_claude_rich_images import registered_macos_root
from run_grok_official_adapter_live import audit_private_settings, copy_private_auth

REPOSITORY = Path(__file__).resolve().parents[2]
TEST = "ai::cli_agent_runtime::grok::multi_skill_live_tests::real_grok_image_skill_hot_add_and_resume"
SCOPE = "grok_image_skill_hot_add_and_resume"
CASES = (("initial", ("alpha",), "RED BLUE"),
         ("hot", ("alpha", "beta"), "GREEN YELLOW"),
         ("cold", ("beta", "alpha"), "BLUE RED"))
require = previous.require


def prepare(workdir, evidence):
    root = workdir / "grok-image-skill"
    root.mkdir(mode=0o700)
    for relative in ("home/.grok", "project/.grok/skills", "unpublished", "state"):
        (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
    (evidence / "fixtures").mkdir(mode=0o700)
    (root / ".infinishell-grok-image-skill-probe").write_text("isolated Grok image skill verification\n")
    fixtures = {}
    for key in ("alpha", "beta"):
        name = "isp-g06-" + key
        target = root / "project/.grok/skills" / name / "SKILL.md"
        initial = target if key == "alpha" else root / "unpublished" / name / "SKILL.md"
        initial.parent.mkdir(mode=0o700)
        marker = "G02_" + key.upper() + "_" + secrets.token_hex(24)
        data = previous.fixture_document(name, marker).replace(
            "按本轮选择顺序，每个标记一行，不加解释。".encode(),
            "按本轮选择顺序，每个标记一行，最后一行输出本轮图片左右色名，不加解释。".encode())
        initial.write_bytes(data)
        initial.chmod(0o400)
        saved = evidence / "fixtures" / (key + ".md")
        saved.write_bytes(data)
        saved.chmod(0o600)
        fixtures[key] = {"name": name, "path": str(target), "initial_path": str(initial),
                         "marker": marker, "sha256": previous.sha(data), "fixture": "fixtures/" + key + ".md"}
    home = root / "home/.grok"
    (home / "config.toml").write_text(previous.CONFIG)
    (home / "config.toml").chmod(0o600)
    # 信任只授予当前合成项目，默认原生审批规则保持不变。
    (home / "trusted_folders.toml").write_text(
        "[folders." + json.dumps(str(root / "project")) + "]\ntrusted = true\ndecided_at = " + str(int(time.time())) + "\n")
    (home / "trusted_folders.toml").chmod(0o600)
    previous.write_json(evidence / "fixtures.safe.json", fixtures)
    return root, fixtures


def environment_for(root, workdir, executable, supervisor):
    allowed = {"HOME", "CODEX_HOME", "PATH", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME", "SHELL",
               "HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY"}
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    require(environment.get("HOME") == os.environ.get("HOME") and bool(environment.get("HOME")), "默认 HOME 不可改变或缺失")
    require(os.environ.get("TMPDIR") == str(workdir), "调用方 TMPDIR 必须是已登记的本轮目录")
    environment.update({"TMPDIR": str(workdir), "GROK_HOME": str(root / "home/.grok"),
        "GROK_DISABLE_API_KEY_AUTH": "1", "GROK_AUTO_UPDATE": "0", "GROK_DISABLE_AUTOUPDATER": "1",
        "GROK_CLAUDE_HOOKS_ENABLED": "0", "GROK_CLAUDE_MCPS_ENABLED": "0",
        "GROK_CODEX_HOOKS_ENABLED": "0", "GROK_CODEX_MCPS_ENABLED": "0",
        "INFINISHELL_GROK_LIVE_ROOT": str(root), "INFINISHELL_GROK_LIVE_EXECUTABLE": str(executable),
        "INFINISHELL_GROK_MANAGED_IMAGE_TRACE": "1", "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(supervisor)})
    return environment


def copy_inputs(args, workdir):
    inputs = workdir / "inputs"
    inputs.mkdir(mode=0o700)
    for source in (args.executable, args.test_binary, args.supervisor):
        require(source.is_absolute() and source.is_file() and not source.is_symlink(), "二进制来源必须是绝对普通文件")
    app = args.supervisor.parents[2]
    require(app.suffix == ".app" and args.supervisor == app / "Contents/MacOS/infinishell", "监督程序必须来自签名应用包")
    executable, test = inputs / "grok", inputs / "warp-libtest"
    for source, target in ((args.executable, executable), (args.test_binary, test)):
        shutil.copy2(source, target)
        target.chmod(0o700)
        require(previous.digest(source) == previous.digest(target), "二进制副本摘要改变")
    copied_app = inputs / "InfiniShellParity.app"
    shutil.copytree(app, copied_app)
    supervisor = copied_app / "Contents/MacOS/infinishell"
    require(previous.digest(args.supervisor) == previous.digest(supervisor), "监督程序副本摘要改变")
    subprocess.run(["/usr/bin/codesign", "-s", "-", "--force", "--timestamp=none", str(test)], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for target, deep in ((test, False), (executable, False), (copied_app, True)):
        subprocess.run(["/usr/bin/codesign", "--verify", "--strict", *(["--deep"] if deep else []), str(target)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return executable, test, supervisor


def entries(events, name):
    return [row for row in events if row.get("event") == name]


def audit_skills(rows, chat, fixtures, keys, text, evidence, turn, native):
    native_text = "\n".join(item.get("text", "") for item in chat[0]["content"])
    expanded, calls, answers, terminals = {}, {}, [], []
    for key in keys:
        info = fixtures[key]
        body = previous.plain_bytes(evidence / info["fixture"]).decode().split("---\n", 2)[2]
        arguments = text if len(keys) == 1 else None
        attributes = ' args="' + arguments + '"' if arguments is not None else ""
        match = re.search('<skill name="' + re.escape(info["name"]) + '"' + re.escape(attributes)
                          + '>\n(.*?)\n</skill>', native_text, re.S)
        if match:
            expected = body + ("\n\n**ARGUMENTS:** " + arguments if arguments is not None else "")
            require(match.group(1) == expected and '<skill name="' + info["name"] + '" path="' + info["path"] + '"/>' in native_text,
                    "原生技能展开正文或路径改变")
            expanded[key] = previous.sha(body.encode())
    for row in rows:
        params, update = row["params"], row["params"]["update"]
        kind = update["sessionUpdate"]
        require(params["sessionId"] == native, "历史混入另一会话")
        if kind in {"tool_call", "tool_call_update", "agent_message_chunk"}:
            require(params.get("_meta", {}).get("promptId") == turn, "工具或输出跨回合")
        if kind == "tool_call":
            key = next((key for key in keys if update.get("rawInput", {}).get("target_file") == fixtures[key]["path"]), None)
            require(key is not None and update.get("_meta", {}).get("x.ai/tool", {}).get("name") == "read_file", "出现未选择技能之外的工具")
            ident = update["toolCallId"]
            require(ident not in calls, "工具身份重复")
            calls[ident] = {"skill": key, "typed_input": False, "completed": False}
        if kind in {"tool_call", "tool_call_update"}:
            require(update["toolCallId"] in calls, "工具更新缺少开始事件")
            call = calls[update["toolCallId"]]
            info = fixtures[call["skill"]]
            if kind == "tool_call_update" and "rawInput" in update:
                require(update["rawInput"] == {"variant": "ReadFile", "target_file": info["path"]}, "原生 read_file 参数改变")
                call["typed_input"] = True
            if update.get("status") == "completed":
                data = update["rawOutput"]["FileContent"]["raw_output"].encode()
                require(data == previous.plain_bytes(evidence / info["fixture"]), "原生技能读取字节改变")
                call.update(completed=True, sha256=previous.sha(data))
        if kind == "turn_completed":
            require(update["prompt_id"] == turn and update["stop_reason"] == "end_turn", "原生回合未正常完成")
            terminals.append(params["_meta"]["eventId"])
        if kind == "agent_message_chunk":
            answers.append(update["content"]["text"])
    require(len(terminals) == 1 and all(call["typed_input"] and call["completed"] for call in calls.values()), "原生工具或终态未闭合")
    require(all(key in expanded or any(call["skill"] == key for call in calls.values()) for key in keys), "未证明本回合实际加载所选技能")
    return expanded, calls, answers, terminals[0]


def audit_turns(root, evidence, events, fixtures, history, projections):
    ready, sent, accepted, done, prepared = [entries(events, name) for name in
        ("ready", "submitted", "accepted", "finished", "attachment_prepared")]
    require(len(ready) == 2 and ready[0]["generation"] != ready[1]["generation"], "缺少两次独立连接")
    native = ready[0]["native_session_id"]
    require(all(row["native_session_id"] == native and row["version"] == "1.0.41"
                and row["model"] == "grok-4.7" and row["policy"] == "inherit" for row in ready), "原生身份、模型或策略改变")
    require(all([row["case"] for row in values] == [case[0] for case in CASES]
                for values in (sent, accepted, done, prepared)), "三轮输入、接收、图片或完成记录不匹配")
    require(len(entries(events, "skill_published")) == len(entries(events, "resume_idle_without_replay")) == 1
                and len(entries(events, "replay_quiet")) == 3, "热新增、恢复或重复输入观察缺失")
    require(len({row["message_id"] for row in sent}) == len({row["turn_id"] for row in done}) == 3, "输入或回合身份重用")
    require(len(projections) == 3, "缺少三个生产最终历史图片投影")
    chat = history["chat_history.jsonl"]
    users = [index for index, row in enumerate(chat) if row.get("type") == "user"]
    require([chat[index].get("prompt_index") for index in users] == [0, 1, 2], "原生历史存在重复或缺失输入")
    updates = history["updates.jsonl"]
    starts = []
    last_index = None
    for index, row in enumerate(updates):
        update = row["params"]["update"]
        if update["sessionUpdate"] == "user_message_chunk":
            current = update.get("_meta", {}).get("promptIndex")
            if current != last_index:
                starts.append(index)
                last_index = current
    require(len(starts) == 3, "原生 typed 输入回合数异常")
    audit = []
    for position, (case, keys, colors) in enumerate(CASES):
        send, ack, end, image = sent[position], accepted[position], done[position], prepared[position]
        require(send["message_id"] == ack["message_id"] == end["message_id"] and ack["turn_id"] == end["turn_id"]
                    and all(row["native_session_id"] == native and row["generation"] == ready[int(position == 2)]["generation"]
                            for row in (send, ack, end)), "适配器消息、代次或原生回合关联改变")
        require(send["stale_generation_rejected"] is True and send["markers_absent_from_input"] is True
                    and all(info["marker"] not in json.dumps(send["input"]) for info in fixtures.values()), "秘密标记泄漏或旧代次未拒绝")
        source = Path(image["source_path"])
        require(source.parent == root / "state/local-cli-attachments" and source.resolve(strict=True) == source,
                "图片持久引用越过本轮附件目录")
        data = previous.plain_bytes(source)
        require(image["mime_type"] == "image/png" and image["byte_count"] == len(data)
                    and image["sha256"] == previous.sha(data) and image["persistent_reference_restored"] is True, "准备图片字节或恢复证据错误")
        rows = updates[starts[position]:starts[position + 1] if position < 2 else len(updates)]
        original = [row["params"]["update"] for row in rows if row["params"]["update"]["sessionUpdate"] == "user_message_chunk"]
        require(all(row["_meta"]["promptIndex"] == position and row["_meta"]["modelId"] == "grok-4.7" for row in original), "原生输入顺序或模型错误")
        text = send["input"]["Submit"]["input"][0]["Text"]
        require(send["input"]["Submit"]["input"] == [{"Text": text}, {"LocalImage": str(source)}]
                    + [{"Skill": {"name": fixtures[key]["name"], "path": fixtures[key]["path"]}} for key in keys],
                "提交附件或技能选择顺序不匹配")
        wire_text = (["/" + fixtures[keys[0]]["name"] + " " + text] if len(keys) == 1
                     else ["/" + fixtures[key]["name"] for key in keys] + [text])
        expected_blocks = [{"type": "text", "text": value} for value in wire_text]
        expected_blocks.append({"type": "image", "mimeType": "image/png", "data": base64.b64encode(data).decode()})
        require(image["native_content_digest"] == previous.sha(json.dumps(expected_blocks[-1], sort_keys=True,
                    separators=(",", ":")).encode()), "原图 typed 编码摘要错误")
        require([row["content"] for row in original] == expected_blocks, "原生 typed 图片或技能输入与提交不一致")
        matched = [row for row in projections if row.get("session_id") == native and row.get("turn_id") == end["turn_id"]
                   and row.get("runtime_generation") == end["generation"]]
        require(len(matched) == 1 and matched[0].get("source") == "verified_native_final_history"
                    and matched[0].get("prompt_index") == position, "图片投影没有来自本回合生产最终历史")
        images = matched[0].get("images", [])
        require(len(images) == 1 and images[0].get("image_sha256") == image["sha256"]
                    and images[0].get("image_bytes") == len(data) and images[0].get("mime_type") == "image/png"
                    and images[0].get("native_content_sha256") == image["native_content_digest"], "最终原生图片字节不符")
        turn_chat = chat[users[position]:users[position + 1] if position < 2 else len(chat)]
        expanded, calls, answers, terminal = audit_skills(rows, turn_chat, fixtures, keys, text, evidence, end["turn_id"], native)
        assistant = [row for row in turn_chat if row.get("type") == "assistant"]
        expected = "\n".join([fixtures[key]["marker"] for key in keys] + [colors])
        require(answers and "".join(answers).strip() == end["output"].strip() and end["outcome"] == "Completed"
                    and assistant and not assistant[-1].get("tool_calls") and assistant[-1].get("content", "").strip() == expected,
                "原生技能标记、图片答案或输出流不符")
        audit.append({"case": case, "message_id": send["message_id"], "turn_id": end["turn_id"], "generation": end["generation"],
                      "image_sha256": image["sha256"], "expanded": expanded, "read_file": calls, "terminal_event_id": terminal,
                      "native_input_exact": True, "final_answer_exact": True})
    require(len({row["sha256"] for row in prepared}) == 3, "三轮必须使用不同原图")
    return audit


def audit_processes(root, workdir, evidence, events, executable):
    ready = entries(events, "ready")
    cleaned = entries(events, "cleanup")
    require(len(ready) == len(cleaned) == 2 and len({row["generation"] for row in ready}) == 2
                and [row["generation"] for row in cleaned] == [row["generation"] for row in ready]
                and ready[0]["native_session_id"] == ready[1]["native_session_id"]
                and all(row.get("resumed") is resumed for values in (ready, cleaned)
                        for row, resumed in zip(values, (False, True))), "清理收据未精确绑定新建与恢复的两代连接")
    result, sockets = [], set()
    for event in cleaned:
        generation = event["generation"]
        directory = root / "state/cli-agent-processes" / generation
        raw = previous.plain_bytes(directory / "manifest.json", 1024 * 1024)
        manifest = json.loads(raw)
        argv = [previous.os_string(value) for value in manifest["arguments"]]
        require(manifest["generation"] == generation and manifest["executable"] == str(executable)
                    and manifest["cwd"] == str(root / "project") and argv.count("--leader") == argv.count("--leader-socket") == 1
                    and "--agent-profile" not in argv and "--no-leader" not in argv, "监督来源不是默认生产独占 leader")
        socket = Path(argv[argv.index("--leader-socket") + 1])
        require(socket.is_absolute() and socket.is_relative_to(workdir) and socket not in sockets and not socket.exists(), "leader socket 身份或清理错误")
        sockets.add(socket)
        receipt = json.loads(previous.plain_bytes(directory / "exit.json", 1024 * 1024))
        require(event["task_finished_ok"] is True and event["extra_turn_events"] == 0 and event["receipt"] == receipt
                    and receipt["generation"] == generation and receipt["manifest_sha256"] == previous.sha(raw)
                    and receipt["cleanup_confirmed"] is True and receipt["exit_code"] == 0, "缺少两代自然退出与真实清理")
        native_raw = previous.plain_bytes(directory / "macos-native.json", 1024 * 1024)
        native = json.loads(native_raw)
        cleanup_raw = previous.plain_bytes(directory / "macos-cleanup.json", 1024 * 1024)
        cleanup = json.loads(cleanup_raw)
        previous.assert_safe(native)
        previous.assert_safe(cleanup)
        require(native["generation"] == cleanup["generation"] == generation and cleanup["native_sha256"] == previous.sha(native_raw)
                    and cleanup["job_removed"] is True and cleanup["resource_cid_destroyed"] is True
                    and cleanup["execution_failed"] is False, "Mac 资源域清理证据不匹配")
        pid = native["identity"]["pid"]
        require(type(pid) is int and pid > 1, "原生 PID 无效")
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            pass
        else:
            raise ValueError("原生 PID 仍存在")
        (evidence / (generation + ".macos-native.raw.json")).write_bytes(native_raw)
        (evidence / (generation + ".macos-cleanup.raw.json")).write_bytes(cleanup_raw)
        result.append({"generation": generation, "receipt": receipt, "native_pid": pid,
                       "macos_native_sha256": previous.sha(native_raw), "macos_cleanup_sha256": previous.sha(cleanup_raw),
                       "native_pid_absent": True})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("workdir", "executable", "credential-home", "test-binary", "supervisor"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    require(sys.platform == "darwin", "仅用于本机 macOS 验收")
    workdir = registered_macos_root(args.workdir)
    output = args.output or workdir / "evidence"
    require(output.parent == workdir and not output.exists() and not output.is_symlink(), "证据目录必须是本轮目录内的新目录")
    output.mkdir(mode=0o700)
    receipt = {"scope": SCOPE, "passed": False, "platform": sys.platform, "version": "1.0.41", "model": "grok-4.7",
               "max_model_inputs": 3, "requested_policy": "inherit", "gui_or_app_restart_verified": False,
               "coordinator_persistence_verified": False, "cross_platform_verified": False,
               "authentication_material_archived": False, "binary_source_binding_requires_build_receipt": True}
    root, fixtures = prepare(workdir, output)
    receipt["workspace"] = str(root)
    auth = None
    copied_auth_identity = None
    source_auth = None
    events = []
    try:
        executable, test, supervisor = copy_inputs(args, workdir)
        source = previous.source_snapshot(REPOSITORY)
        receipt.update(source, runner_sha256=previous.digest(Path(__file__).resolve()),
                       test_binary_sha256=previous.digest(test), supervisor_sha256=previous.digest(supervisor))
        native = verify_binary(executable, current_platform("1.0.41"), "1.0.41")
        receipt["native"] = native
        environment = environment_for(root, workdir, executable, supervisor)
        version = subprocess.check_output([str(executable), "--version"], cwd=root / "project", env=environment, text=True, timeout=20).strip()
        require(version == previous.VERSION, "实际 CLI 版本改变")
        listing = subprocess.check_output([str(test), "--list"], cwd=REPOSITORY, env=environment, text=True, timeout=45)
        require(TEST + ": test" in listing, "二进制缺少图片技能验收入口")
        source_auth = previous.auth_identity(args.credential_home / "auth.json")
        settings = args.credential_home / "config.toml"
        source_settings_sha = previous.digest(settings) if settings.exists() else None
        config_before = previous.plain_bytes(root / "home/.grok/config.toml")
        auth = root / "home/.grok/auth.json"
        require(not auth.exists() and not auth.is_symlink(), "私有认证目标已存在")
        try:
            require(copy_private_auth(args.credential_home, root / "home/.grok") == auth, "私有认证路径改变")
        finally:
            # 复制中途失败也登记已创建的私有文件，finally 按身份删除残留。
            if auth.exists():
                previous.auth_identity(auth)
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
        raw_events = previous.plain_bytes(root / "events.ndjson")
        events = [json.loads(line) for line in raw_events.splitlines() if line]
        previous.assert_safe(events)
        text = previous.plain_bytes(log_path).decode("utf-8", "replace")
        receipt.update(raw_events_sha256=previous.sha(raw_events), raw_log_sha256=previous.digest(log_path))
        ready = entries(events, "ready")
        require(ready, "未取得原生会话")
        history = previous.capture_native(root, output, ready[0]["native_session_id"])
        require(process.returncode == 0 and re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", text)
                    and events[-1].get("event") == "adapter_flow_passed" and events[-1].get("scope") == SCOPE,
                "真实适配器验收未通过")
        projections = [json.loads(line.split("GROK_NATIVE_IMAGE_HISTORY ", 1)[1]) for line in text.splitlines()
                       if "GROK_NATIVE_IMAGE_HISTORY " in line]
        turns = audit_turns(root, output, events, fixtures, history, projections)
        processes = audit_processes(root, workdir, output, events, executable)
        previous.write_json(output / "native-audit.safe.json", {"turns": turns, "processes": processes,
                            "native_images": projections, "native_inputs": 3, "native_generations": 2})
        require(previous.auth_identity(args.credential_home / "auth.json") == source_auth
                    and (previous.digest(settings) if settings.exists() else None) == source_settings_sha, "原认证或设置变化")
        receipt["private_settings_audit"] = audit_private_settings(config_before, previous.plain_bytes(root / "home/.grok/config.toml"))
        require(receipt["private_settings_audit"]["settings_scope_verified"], "私有原生设置超出允许变动")
        require(all(previous.digest(Path(info["path"])) == info["sha256"] for info in fixtures.values()), "技能夹具改变")
        require(previous.source_snapshot(REPOSITORY)["source_files_sha256"] == source["source_files_sha256"]
                    and previous.digest(test) == receipt["test_binary_sha256"]
                    and previous.digest(supervisor) == receipt["supervisor_sha256"]
                    and verify_binary(executable, current_platform("1.0.41"), "1.0.41") == native, "运行中源码或二进制改变")
        receipt.update(passed=True, authenticated_native_history_verified=True, native_input_count=3,
                       natural_cleanup_generations=2, original_auth_and_settings_unchanged=True)
    except Exception as error:
        receipt.update(failure_type=type(error).__name__, failure=previous.SECRET_TEXT.sub("[已移除认证材料]", str(error)))
    finally:
        if auth is not None:
            try:
                if auth.exists() or auth.is_symlink():
                    current = auth.lstat()
                    require(copied_auth_identity is not None and not auth.is_symlink() and current.st_uid == os.getuid()
                                and (current.st_dev, current.st_ino) == (copied_auth_identity.st_dev, copied_auth_identity.st_ino),
                            "认证副本身份改变，保留供精确清理")
                    auth.unlink()
                receipt["private_auth_copy_removed"] = True
            except Exception as error:
                receipt.update(passed=False, private_auth_copy_removed=False, auth_cleanup_failure_type=type(error).__name__)
        if source_auth is not None:
            try:
                unchanged = (previous.auth_identity(args.credential_home / "auth.json") == source_auth
                    and (previous.digest(settings) if settings.exists() else None) == source_settings_sha)
                receipt["original_auth_and_settings_unchanged"] = unchanged
                receipt["passed"] = receipt["passed"] and unchanged
            except Exception:
                receipt.update(passed=False, original_auth_and_settings_unchanged=False)
        # 失败轮也保留已有原生会话的原始选中行；不清理工作区、进程或唯一证据。
        try:
            if not events and (root / "events.ndjson").exists():
                events = [json.loads(line) for line in previous.plain_bytes(root / "events.ndjson").splitlines() if line]
            ready = entries(events, "ready")
            if ready and not (output / "native-chat_history.jsonl").exists():
                previous.capture_native(root, output, ready[0]["native_session_id"])
            if (root / "events.ndjson").exists():
                previous.assert_safe(events)
                (output / "events.raw.ndjson").write_bytes(previous.plain_bytes(root / "events.ndjson"))
            if (root / "private-test-output.txt").exists():
                raw_log = previous.plain_bytes(root / "private-test-output.txt")
                text = raw_log.decode("utf-8", "replace")
                previous.assert_safe(text)
                require(re.search(r'"(?:access_token|refresh_token|id_token|api_key|authorization|password)"\s*:', text, re.I) is None,
                        "原日志疑似包含认证字段，保留私有原件并禁止归档")
                (output / "test-output.raw.log").write_bytes(raw_log)
        except Exception:
            receipt["partial_history_capture_failed"] = True
            receipt["passed"] = False
        previous.write_json(output / "receipt.safe.json", receipt)
        previous.index_output(output)
    print(json.dumps({"scope": SCOPE, "passed": receipt["passed"], "receipt": str(output / "receipt.safe.json")}, ensure_ascii=False))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
