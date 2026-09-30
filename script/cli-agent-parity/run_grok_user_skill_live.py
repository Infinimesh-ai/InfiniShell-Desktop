#!/usr/bin/env python3
"""固定 Grok 用户与项目技能的真实验收；仅在已登记短目录内使用私有 GROK_HOME。"""

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time

import run_grok_image_skill_live as shared
import run_grok_multi_skill_live as previous
from prepare_grok_cli import current_platform, verify_binary
from probe_claude_rich_images import registered_macos_root
from run_grok_official_adapter_live import audit_private_settings, copy_private_auth

REPOSITORY = Path(__file__).resolve().parents[2]
TEST = "ai::cli_agent_runtime::grok::multi_skill_live_tests::real_grok_user_skill_hot_add_and_resume"
SCOPE = "grok_user_skill_hot_add_and_resume"
CASES = (("initial", ("alpha", "beta")), ("hot", ("gamma",)), ("cold", ("gamma", "alpha")))
require = previous.require
entries = shared.entries


def source_snapshot():
    source = previous.source_snapshot(REPOSITORY)
    for name in ("grok_profile_skills.rs", "grok_profile_skills_tests.rs"):
        path = "app/src/ai/cli_agent_runtime/" + name
        source["source_files_sha256"][path] = previous.digest(REPOSITORY / path)
    source["runner_sha256"] = previous.digest(Path(__file__).resolve())
    return source


def prepare(workdir, evidence):
    root = workdir / "grok-user-skill"
    root.mkdir(mode=0o700)
    for relative in ("home/.grok/skills", "project/.grok/skills", "unpublished", "state"):
        (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
    (evidence / "fixtures").mkdir(mode=0o700)
    (root / ".infinishell-grok-user-skill-probe").write_text("isolated Grok user skill verification\n")
    fixtures = {}
    for key in ("alpha", "beta", "gamma"):
        name = "isp-g06-" + key
        scope = "local" if key == "alpha" else "user"
        directory = root / ("project/.grok/skills" if scope == "local" else "home/.grok/skills")
        target = directory / name / "SKILL.md"
        initial = root / "unpublished" / name / "SKILL.md" if key == "gamma" else target
        initial.parent.mkdir(mode=0o700)
        marker = "G06_" + key.upper() + "_" + secrets.token_hex(24)
        data = previous.fixture_document(name, marker)
        initial.write_bytes(data)
        initial.chmod(0o400)
        saved = evidence / "fixtures" / (key + ".md")
        saved.write_bytes(data)
        saved.chmod(0o600)
        fixtures[key] = {"name": name, "scope": scope, "qualified_name": scope + ":" + name,
                         "path": str(target), "initial_path": str(initial), "marker": marker,
                         "sha256": previous.sha(data), "bytes": len(data), "fixture": "fixtures/" + key + ".md"}
    home = root / "home/.grok"
    (home / "config.toml").write_text(previous.CONFIG)
    (home / "config.toml").chmod(0o600)
    # 私有用户技能不改变默认 HOME；信任只授予本轮合成项目，权限保持 Inherit。
    (home / "trusted_folders.toml").write_text(
        "[folders." + json.dumps(str(root / "project")) + "]\ntrusted = true\ndecided_at = " + str(int(time.time())) + "\n")
    (home / "trusted_folders.toml").chmod(0o600)
    previous.write_json(evidence / "fixtures.safe.json", fixtures)
    return root, fixtures


def environment_for(root, workdir, executable, supervisor):
    environment = shared.environment_for(root, workdir, executable, supervisor)
    environment.pop("INFINISHELL_GROK_MANAGED_IMAGE_TRACE")
    environment["INFINISHELL_GROK_USER_SKILL_TRACE"] = "1"
    return environment


def audit_catalogs(traces, fixtures, ready):
    phases = ["ready_catalog", "reload", "refreshed_catalog", "ready_catalog"]
    require([row.get("phase") for row in traces] == phases, "缺少精确初始目录、reload、刷新目录或冷恢复目录")
    catalogs = []
    for index, row in enumerate(traces):
        connection = ready[int(index == 3)]
        require(row.get("generation") == connection["generation"]
                and row.get("native_session_id") == connection["native_session_id"], "原生目录跨代次或会话")
        if row["phase"] == "reload":
            require(row.get("proof") == {"reloaded": 1} and type(row["proof"]["reloaded"]) is int,
                    "独占 leader reload 未精确确认一个会话")
            continue
        keys = ("alpha", "beta") if index == 0 else ("alpha", "beta", "gamma")
        commands = row.get("proof", {}).get("commands")
        require(isinstance(commands, list) and len(commands) == len(keys), "目录缺少技能或混入未知技能")
        catalog = {}
        for key in keys:
            info = fixtures[key]
            matches = [command for command in commands if command.get("_meta", {}).get("bareName") == info["name"]]
            require(len(matches) == 1, "原生技能目录有歧义")
            command = matches[0]
            require(command.get("_meta") == {"bareName": info["name"], "scope": info["scope"],
                        "qualifiedName": info["qualified_name"], "path": info["path"]}
                    and command.get("name") in {info["name"], info["qualified_name"]}
                    and "input" in command and command["input"] is None, "技能来源、限定名称或原路径不符")
            catalog[key] = command
        catalogs.append(catalog)
    return catalogs


def audit_turns(root, evidence, events, fixtures, history, traces):
    ready, sent, accepted, done = [entries(events, name) for name in ("ready", "submitted", "accepted", "finished")]
    require(len(ready) == 2 and ready[0]["generation"] != ready[1]["generation"]
            and [row.get("resumed") for row in ready] == [False, True], "缺少新建与冷恢复的两次连接")
    native = ready[0]["native_session_id"]
    require(all(row["native_session_id"] == native and row["version"] == "1.0.41"
                and row["model"] == "grok-4.7" and row["policy"] == "inherit" for row in ready), "原生身份、模型或策略改变")
    require(all([row["case"] for row in values] == [case for case, keys in CASES]
                for values in (sent, accepted, done)), "三轮输入、ACK 或完成记录不匹配")
    catalogs = audit_catalogs(traces, fixtures, ready)
    published = entries(events, "skill_published")
    require(len(published) == len(entries(events, "resume_idle_without_replay")) == 1
                and published[0]["path"] == fixtures["gamma"]["path"]
                and published[0]["generation"] == ready[0]["generation"]
                and published[0]["native_session_id"] == native and published[0]["after_case"] == "initial"
                and published[0]["same_connection"] is True, "用户技能热新增或冷恢复观察缺失")
    require(len({row["message_id"] for row in sent}) == len({row["turn_id"] for row in done}) == 3, "消息或回合身份重用")
    require([(row["case"], row["message_id"]) for row in entries(events, "replay_quiet")]
                == [(row["case"], row["message_id"]) for row in sent], "重复投递观察未绑定原消息")
    chat = history["chat_history.jsonl"]
    users = [index for index, row in enumerate(chat) if row.get("type") == "user"]
    require([chat[index].get("prompt_index") for index in users] == [0, 1, 2], "原生历史存在重复或缺失输入")
    updates = history["updates.jsonl"]
    starts, last_index = [], None
    for index, row in enumerate(updates):
        update = row["params"]["update"]
        if update["sessionUpdate"] == "user_message_chunk":
            current = update.get("_meta", {}).get("promptIndex")
            if current != last_index:
                starts.append(index)
                last_index = current
    require(len(starts) == 3, "原生 typed 输入回合数异常")
    audit = []
    for position, (case, keys) in enumerate(CASES):
        send, ack, end = sent[position], accepted[position], done[position]
        require(send["message_id"] == ack["message_id"] == end["message_id"] and ack["turn_id"] == end["turn_id"]
                    and all(row["native_session_id"] == native and row["generation"] == ready[int(position == 2)]["generation"]
                            for row in (send, ack, end)), "适配器消息、代次或原生回合关联改变")
        require(send["stale_generation_rejected"] is True and send["markers_absent_from_input"] is True
                    and all(info["marker"] not in json.dumps(send["input"]) for info in fixtures.values()), "秘密标记泄漏或旧代次未拒绝")
        rows = updates[starts[position]:starts[position + 1] if position < 2 else len(updates)]
        original = [row["params"]["update"] for row in rows if row["params"]["update"]["sessionUpdate"] == "user_message_chunk"]
        require(all(row["_meta"]["promptIndex"] == position and row["_meta"]["modelId"] == "grok-4.7" for row in original), "原生输入顺序或模型错误")
        text = send["input"]["Submit"]["input"][0]["Text"]
        require(send["input"]["Submit"]["input"] == [{"Text": text}]
                    + [{"Skill": {"name": fixtures[key]["name"], "path": fixtures[key]["path"]}} for key in keys],
                "提交用户或项目技能的路径及顺序不匹配")
        catalog = catalogs[position]
        wire = (["/" + catalog[keys[0]]["name"] + " " + text] if len(keys) == 1
                else ["/" + catalog[key]["name"] for key in keys] + [text])
        if len(keys) > 1:
            wire.append(json.dumps({"selected_skills": [
                {"qualifiedName": catalog[key]["_meta"]["qualifiedName"], "path": catalog[key]["_meta"]["path"]}
                for key in keys]}, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
        require([row["content"] for row in original] == [{"type": "text", "text": value} for value in wire],
                "原生 typed 技能命令与经确认目录不一致")
        turn_chat = chat[users[position]:users[position + 1] if position < 2 else len(chat)]
        expanded, calls, answers, terminal = shared.audit_skills(rows, turn_chat, fixtures, keys, text, evidence, end["turn_id"], native)
        require(keys[0] in expanded and set(keys[1:]).issubset({call["skill"] for call in calls.values()}),
                "未证明首技能原生展开且其余技能由本回合精确 read_file 加载")
        assistant = [row for row in turn_chat if row.get("type") == "assistant"]
        expected = "\n".join(fixtures[key]["marker"] for key in keys)
        require(answers and "".join(answers).strip() == end["output"].strip() and end["outcome"] == "Completed"
                    and assistant and not assistant[-1].get("tool_calls") and assistant[-1].get("content", "").strip() == expected,
                "用户技能实际答案、顺序或输出流不符")
        audit.append({"case": case, "message_id": send["message_id"], "turn_id": end["turn_id"], "generation": end["generation"],
                      "selected_sources": [fixtures[key]["qualified_name"] for key in keys], "expanded": expanded,
                      "read_file": calls, "terminal_event_id": terminal, "native_input_exact": True, "final_answer_exact": True})
    return audit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("workdir", "executable", "credential-home", "test-binary", "supervisor"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--build-source-binding", type=Path, required=True)
    args = parser.parse_args()
    require(sys.platform == "darwin", "仅用于本机 macOS 验收")
    workdir = registered_macos_root(args.workdir)
    output = args.output or workdir / "evidence"
    require(output.parent == workdir and not output.exists() and not output.is_symlink(), "证据目录必须是本轮目录内的新目录")
    output.mkdir(mode=0o700)
    receipt = {"scope": SCOPE, "passed": False, "platform": sys.platform, "version": "1.0.41", "model": "grok-4.7",
               "max_model_inputs": 3, "requested_policy": "inherit", "gui_or_app_restart_verified": False,
               "coordinator_persistence_verified": False, "cross_platform_verified": False,
               "authentication_material_archived": False, "binary_source_binding_requires_build_receipt": True,
               "parent_permission_ceiling": None, "profile_override": False, "permission_rules_override": False}
    root, fixtures = prepare(workdir, output)
    receipt["workspace"] = str(root)
    auth = None
    copied_auth_identity = None
    source_auth = None
    events = []
    try:
        executable, test, supervisor = shared.copy_inputs(args, workdir)
        source = source_snapshot()
        build = json.loads(previous.plain_bytes(args.build_source_binding, 2 * 1024 * 1024))
        require(build["baseline_commit"] == source["baseline_commit"]
                and all(build["source_files_sha256"].get(name) == digest for name, digest in source["source_files_sha256"].items())
                and build["test_binary_sha256"] == previous.digest(args.test_binary)
                and build["supervisor_sha256"] == previous.digest(args.supervisor), "构建声明、当前源码或原二进制摘要不符")
        receipt.update(build_source_binding_sha256=previous.digest(args.build_source_binding),
                       build_manifest_supplied_by_integrator=True)
        receipt.update(source, runner_sha256=previous.digest(Path(__file__).resolve()),
                       test_binary_sha256=previous.digest(test), supervisor_sha256=previous.digest(supervisor))
        native = verify_binary(executable, current_platform("1.0.41"), "1.0.41")
        receipt["native"] = native
        environment = environment_for(root, workdir, executable, supervisor)
        version = subprocess.check_output([str(executable), "--version"], cwd=root / "project", env=environment, text=True, timeout=20).strip()
        require(version == previous.VERSION, "实际 CLI 版本改变")
        listing = subprocess.check_output([str(test), "--list"], cwd=REPOSITORY, env=environment, text=True, timeout=45)
        require(TEST + ": test" in listing, "二进制缺少用户技能验收入口")
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
        traces = [json.loads(line.split("GROK_USER_SKILL_CATALOG ", 1)[1]) for line in text.splitlines()
                  if "GROK_USER_SKILL_CATALOG " in line]
        turns = audit_turns(root, output, events, fixtures, history, traces)
        processes = shared.audit_processes(root, workdir, output, events, executable)
        previous.write_json(output / "native-audit.safe.json", {"turns": turns, "processes": processes,
                            "native_catalogs": traces, "native_inputs": 3, "native_generations": 2})
        require(previous.auth_identity(args.credential_home / "auth.json") == source_auth
                    and (previous.digest(settings) if settings.exists() else None) == source_settings_sha, "原认证或设置变化")
        receipt["private_settings_audit"] = audit_private_settings(config_before, previous.plain_bytes(root / "home/.grok/config.toml"))
        require(receipt["private_settings_audit"]["settings_scope_verified"], "私有原生设置超出允许变动")
        require(all(previous.digest(Path(info["path"])) == info["sha256"] for info in fixtures.values()), "技能夹具改变")
        require(source_snapshot()["source_files_sha256"] == source["source_files_sha256"]
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
