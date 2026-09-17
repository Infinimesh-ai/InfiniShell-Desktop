#!/usr/bin/env python3
"""固定 Claude 无模型权限查询：仅自有配置、精确控制请求及安全投影，不授权产品派发。"""

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import queue
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
from prepare_claude_cli import (VERSION, current_platform, isolated_environment,
                                regular_file, require, verify_binary, verify_version)
from probe_claude_no_credentials import Recorder, command

TIMEOUT = 15
MODES = {"default", "dontAsk", "plan", "acceptEdits", "auto", "bypassPermissions"}
SOURCES = {"userSettings", "projectSettings", "localSettings", "flagSettings", "policySettings"}
RULE_SOURCES = SOURCES | {"cliArg", "command", "session", "toolsNarrowing", "mcpServerPolicy", "hostCredential"}
PERMISSIONS = {"allow": "strings", "deny": "strings", "ask": "strings", "defaultMode": "string",
               "additionalDirectories": "strings", "disableBypassPermissionsMode": "string"}
SANDBOX = {"enabled": "bool", "failIfUnavailable": "bool", "autoAllowBashIfSandboxed": "bool",
           "allowUnsandboxedCommands": "bool"}
BASE = {"permissions": {"allow": ["Read(./allowed/**)"], "deny": ["Edit(./blocked/**)"], "ask": ["Bash"]},
        "sandbox": {"enabled": False, "failIfUnavailable": True,
                    "autoAllowBashIfSandboxed": False, "allowUnsandboxedCommands": False}}
ALLOWED_REQUESTS = {"initialize", "get_settings", "list_permission_rules", "set_permission_mode",
                    "apply_flag_settings", "update_settings"}


def unknown(value, expected, path, flags):
    require(isinstance(value, dict), "原生权限对象形状不匹配")
    count = len(set(value) - set(expected))
    if count:
        # 未知键名和值都可能包含秘密；仅记所在已知对象及数量，不能默默丢弃。
        flags.append({"path": path, "unknown_field_count": count})


def fields(value, schema, path, flags):
    unknown(value, schema, path, flags)
    result = {}
    for key, kind in schema.items():
        if key not in value:
            continue
        item = value[key]
        valid = ((kind == "bool" and type(item) is bool) or
                 (kind == "string" and isinstance(item, str)) or
                 (kind == "strings" and isinstance(item, list) and all(isinstance(x, str) for x in item)))
        require(valid, "原生权限字段类型不匹配")
        result[key] = copy.deepcopy(item)
    return result


def settings_projection(value, path, flags):
    unknown(value, {"permissions", "sandbox"}, path, flags)
    result = {}
    for key, schema in (("permissions", PERMISSIONS), ("sandbox", SANDBOX)):
        if key in value:
            result[key] = fields(value[key], schema, f"{path}.{key}", flags)
    return result


def project_settings(value):
    flags = []
    unknown(value, {"effective", "sources", "applied", "errors"}, "get_settings", flags)
    require(isinstance(value.get("sources"), list), "缺少设置来源数组")
    result = {"effective": settings_projection(value.get("effective"), "effective", flags), "sources": []}
    for source in value["sources"]:
        unknown(source, {"source", "settings"}, "sources[]", flags)
        require(source.get("source") in SOURCES, "未知设置来源")
        result["sources"].append({"source": source["source"],
                                  "settings": settings_projection(source.get("settings"), "source.settings", flags)})
    require(len({x["source"] for x in result["sources"]}) == len(result["sources"]), "设置来源重复")
    result["reported_errors"] = bool(value.get("errors"))
    result["unknown_fields"] = flags
    # applied 的模型/账号周边信息与权限证明无关，不把它写入记录。
    return result


def project_rules(value):
    flags = []
    unknown(value, {"state"}, "list_permission_rules", flags)
    state = value.get("state")
    unknown(state, {"rules", "workspaceDirectories", "originalCwd", "managedOnly", "errors"}, "state", flags)
    require(isinstance(state.get("rules"), list) and isinstance(state.get("workspaceDirectories"), list),
            "原生规则或目录数组缺失")
    require(type(state.get("managedOnly")) is bool and isinstance(state.get("originalCwd"), str),
            "缺少 managedOnly 或 originalCwd")
    result = {"rules": [], "workspaceDirectories": [], "originalCwd": state["originalCwd"],
              "managedOnly": state["managedOnly"], "reported_errors": bool(state.get("errors"))}
    for rule in state["rules"]:
        unknown(rule, {"behavior", "source", "rule", "editability", "notInEffect", "description"}, "state.rules[]", flags)
        require(rule.get("behavior") in {"allow", "deny", "ask"} and rule.get("source") in RULE_SOURCES
                and isinstance(rule.get("rule"), str) and rule.get("editability") in {"readonly", "persistent", "session"}
                and type(rule.get("notInEffect", False)) is bool, "原生规则形状不匹配")
        result["rules"].append({key: rule[key] for key in ("behavior", "source", "rule", "editability", "notInEffect") if key in rule})
    for directory in state["workspaceDirectories"]:
        unknown(directory, {"path", "source"}, "state.workspaceDirectories[]", flags)
        require(isinstance(directory.get("path"), str) and directory.get("source") in RULE_SOURCES, "原生目录形状不匹配")
        result["workspaceDirectories"].append({"path": directory["path"], "source": directory["source"]})
    result["unknown_fields"] = flags
    return result


def consistency(snapshot):
    settings, state = snapshot["settings"], snapshot["rules"]
    reasons = []
    if settings["unknown_fields"] or state["unknown_fields"]:
        reasons.append("unknown_fields")
    if settings["reported_errors"] or state["reported_errors"]:
        reasons.append("native_errors")
    if state["managedOnly"]:
        reasons.append("managed_only_unverified")
    expected = sorted((source["source"], behavior, rule) for source in settings["sources"]
                      for behavior in ("allow", "deny", "ask")
                      for rule in source["settings"].get("permissions", {}).get(behavior, []))
    actual = sorted((rule["source"], rule["behavior"], rule["rule"]) for rule in state["rules"] if rule["source"] in SOURCES)
    if actual != expected or any(rule.get("notInEffect") for rule in state["rules"]):
        reasons.append("settings_live_rules_mismatch")
    if settings["effective"].get("sandbox", {}).get("enabled") is not False:
        reasons.append("sandbox_not_explicitly_disabled")
    if snapshot["mode_before"] != snapshot["mode_after"]:
        reasons.append("mode_changed_during_queries")
    return reasons


def comparable(snapshot):
    return {key: snapshot[key] for key in ("mode_after", "settings", "rules")}


def compare(parent, child):
    reasons = consistency(parent) + consistency(child)
    if comparable(parent) != comparable(child):
        reasons.append("observed_projection_differs")
    return {"equal_and_consistent_observation": not reasons, "reasons": sorted(set(reasons)),
            "atomic_permission_ceiling_proven": False, "dispatch_authorized": False}


def scrub(value, root):
    if isinstance(value, dict):
        return {key: scrub(item, root) for key, item in value.items()}
    if isinstance(value, list):
        return [scrub(item, root) for item in value]
    if isinstance(value, str):
        return value.replace(str(root), "<isolated-probe>")
    return value


class Session:
    def __init__(self, executable, env, cwd, report, settings=BASE, sources="", additional=None, mode=None):
        args = command(executable)
        args[args.index("--setting-sources") + 1] = sources
        args += ["--settings", json.dumps(settings)]
        if additional:
            args += ["--add-dir", str(additional)]
        if mode:
            require(mode == "dontAsk", "探针只允许显式收紧的启动模式")
            args += ["--permission-mode", mode]
        self.recorder = Recorder(args, env, cwd)
        self.report = report
        report.update(native_pid=self.recorder.process.pid, queries=[], notifications=[], eof=None)
        self.sequence = 0
        self.native_session_id = None
        self.answered = set()

    def notification(self, message):
        require(isinstance(message, dict) and message.get("type") == "system", "拒绝用户、模型或未知输出")
        subtype = message.get("subtype")
        common = {"type", "subtype", "session_id", "uuid"}
        if subtype == "status":
            require(set(message) <= common | {"status", "permissionMode"} and message.get("status") is None
                    and message.get("permissionMode") in MODES, "未知状态通知")
            projected = {"subtype": subtype, "permissionMode": message["permissionMode"]}
        elif subtype == "background_tasks_changed":
            require(set(message) <= common | {"tasks"} and message.get("tasks") == [], "意外原生后台任务")
            projected = {"subtype": subtype, "tasks": []}
        else:
            raise ValueError("未验证的原生通知，拒绝计为通过")
        native_id = message.get("session_id")
        require(isinstance(native_id, str) and native_id, "通知没有原生连接身份")
        require(self.native_session_id is None or self.native_session_id == native_id, "原生通知会话身份改变")
        self.native_session_id = native_id
        projected.update(native_session_id=native_id, uuid=message.get("uuid"))
        self.report["notifications"].append(projected)

    def query(self, subtype, **kwargs):
        require(subtype in ALLOWED_REQUESTS, "不允许此控制请求")
        if subtype == "set_permission_mode":
            require(kwargs == {"mode": "dontAsk"} or kwargs == {"mode": "default"}, "禁止探针放宽初始权限上限")
        request_id = f"infinishell-permissions-{uuid.uuid4()}-{self.sequence}"
        self.sequence += 1
        message = {"type": "control_request", "request_id": request_id, "request": {"subtype": subtype, **kwargs}}
        self.report["queries"].append({"subtype": subtype, "request_id": request_id, "success": False})
        row = self.report["queries"][-1]
        self.recorder.process.stdin.write(json.dumps(message) + "\n")
        self.recorder.process.stdin.flush()
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            require(not self.recorder.reader_errors, "原生输出读取失败")
            try:
                output = self.recorder.messages.get(timeout=0.1)
            except queue.Empty:
                require(self.recorder.process.poll() is None, "原生查询响应前进程退出")
                continue
            require(isinstance(output, dict), "原生 stdout 不是结构化消息")
            if output.get("type") != "control_response":
                self.notification(output)
                continue
            response = output.get("response", {})
            require(response.get("request_id") == request_id and request_id not in self.answered, "重复或错误关联的控制响应")
            self.answered.add(request_id)
            if response.get("subtype") == "error":
                # 仅这个受控负向错误可写入；原生任意错误正文可能带设置秘密。
                expected = "update_settings keys not allowed: permissions"
                require(subtype == "update_settings" and response.get("error") == expected, "原生请求失败，正文已省略")
                row["expected_error"] = expected
                return None
            require(response.get("subtype") == "success", "未知控制响应状态")
            row["success"] = True
            return response.get("response", {})
        raise ValueError("原生控制请求超时")

    def initialize(self):
        value = self.query("initialize")
        require(value.get("pid") == self.recorder.process.pid and type(value.get("pid")) is int
                and value.get("session_state") == "idle" and value.get("current_permission_mode") in MODES,
                "初始化没有绑定本次空闲进程和实际模式")
        account = value.get("account", {})
        require(account.get("tokenSource") == "none" and account.get("apiProvider") == "firstParty", "必须无账号和默认提供方")
        return value["current_permission_mode"]

    def snapshot(self, name):
        before = self.initialize()
        settings = project_settings(self.query("get_settings"))
        rules = project_rules(self.query("list_permission_rules"))
        after = self.initialize()
        snapshot = {"name": name, "native_pid": self.recorder.process.pid, "mode_before": before,
                    "mode_after": after, "settings": settings, "rules": rules}
        snapshot["consistency_rejections"] = consistency(snapshot)
        self.report.setdefault("snapshots", []).append(snapshot)
        return snapshot

    def close(self):
        try:
            self.report["eof"] = self.recorder.finish_eof()
        finally:
            self.report["forced_cleanup_used"] = self.recorder.close()
        while not self.recorder.messages.empty():
            self.notification(self.recorder.messages.get_nowait())
        for row in self.recorder.records:
            if row["direction"] == "stderr":
                require(not isinstance(row["message"], dict) or row["message"].get("type") not in
                        {"user", "assistant", "result", "stream_event"}, "stderr 出现模型消息")
        self.report["stderr_records_omitted"] = sum(x["direction"] == "stderr" for x in self.recorder.records)
        eof = self.report["eof"]
        require(eof["stdin_eof_exited_within_5s"] and eof["exit_code_before_cleanup"] == 0
                and not self.report["forced_cleanup_used"], "EOF 没有正常自行退出")


def write_settings(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value), encoding="utf-8")


def run(executable, root, report):
    report["binary"] = verify_binary(executable, current_platform())
    report["cli_version"] = verify_version(executable, root)
    env = isolated_environment(root)
    project, additional = root / "project 中文", root / "additional 目录"
    project.mkdir(); additional.mkdir()
    report["sessions"] = []

    def start(**kwargs):
        row = {}; report["sessions"].append(row)
        return Session(executable, env, project, row, additional=additional, **kwargs)

    parent = start()
    try:
        original = parent.snapshot("parent_initial")
        require(not consistency(original), "受控初始权限观察不一致")
        require(original["rules"]["originalCwd"] == str(project)
                and original["rules"]["workspaceDirectories"] == [{"path": str(additional), "source": "cliArg"}],
                "原生 cwd 或 additional directory 不匹配本次控制路径")
        child = start()
        try:
            child_initial = child.snapshot("child_initial")
        finally:
            child.close()
        initial_comparison = compare(original, child_initial)
        require(initial_comparison["equal_and_consistent_observation"], "相同受控父子投影不相等")
        require(parent.query("set_permission_mode", mode="dontAsk") == {"mode": "dontAsk"}, "模式收紧没有实际回执")
        tightened = parent.snapshot("parent_dont_ask")
        require(tightened["mode_after"] == "dontAsk", "同连接没有刷新实际模式")
        mode_comparison = compare(tightened, child_initial)
        require(not mode_comparison["equal_and_consistent_observation"], "旧子模式没有被拒绝")
        require(parent.query("set_permission_mode", mode="default") == {"mode": "default"}, "恢复原模式失败")
        parent.query("apply_flag_settings", settings={"permissions": {"allow": [], "deny": ["Read(./allowed/**)"]}})
        merged = parent.snapshot("parent_merged_deny")
        permissions = merged["settings"]["effective"]["permissions"]
        require(permissions["allow"] == BASE["permissions"]["allow"] and "Read(./allowed/**)" in permissions["deny"]
                and not consistency(merged), "flag 合并与 live 规则不符合实测契约")
        merge_comparison = compare(merged, child_initial)
        require(not merge_comparison["equal_and_consistent_observation"], "旧子规则没有被拒绝")
        fresh_child = start(settings=merged["settings"]["effective"])
        try:
            refreshed = fresh_child.snapshot("child_current_flags")
        finally:
            fresh_child.close()
        fresh_comparison = compare(merged, refreshed)
        require(fresh_comparison["equal_and_consistent_observation"], "新子受控 flag 投影不匹配当前父")
        report["comparisons"] = {"initial": initial_comparison, "stale_mode": mode_comparison,
                                 "stale_rules": merge_comparison, "refreshed_flags": fresh_comparison}
    finally:
        parent.close()

    user_file, project_file, local_file = root / "claude/settings.json", project / ".claude/settings.json", project / ".claude/settings.local.json"
    write_settings(user_file, {"permissions": {"allow": ["Read(./user-only/**)"]}})
    write_settings(project_file, {"permissions": {"deny": ["Edit(./project-only/**)"]}})
    local_original = {"permissions": {"ask": ["Read(./local-only/**)"]}}
    write_settings(local_file, local_original)
    sourced = start(sources="user,project,local")
    try:
        before = sourced.snapshot("sourced_before_drift")
        require(not consistency(before) and [x["source"] for x in before["settings"]["sources"]] ==
                ["userSettings", "projectSettings", "localSettings", "flagSettings"], "四种设置来源未被确认")
        rejected = sourced.query("update_settings", source="localSettings", settings={"permissions": {"deny": ["Read(./api-rejected/**)"]}})
        require(rejected is None and json.loads(local_file.read_text()) == local_original, "受限设置写入未被拒绝或改写了文件")
        write_settings(local_file, {"permissions": {"deny": ["Read(./file-drift/**)"]}})
        # 这段有界轮询只观察变更传播，不作为原子快照或无未来变更的证明。
        deadline = time.monotonic() + 3
        while True:
            drifted = sourced.snapshot("sourced_after_file_change")
            if "Read(./file-drift/**)" in drifted["settings"]["effective"]["permissions"].get("deny", []):
                break
            require(time.monotonic() < deadline, "期限内 get_settings 没有观察到自有文件变更")
            time.sleep(0.1)
        report["file_drift"] = {"effective_observed_change": True,
                                "live_rules_observed_change": any(x["rule"] == "Read(./file-drift/**)" for x in drifted["rules"]["rules"]),
                                "consistency_rejections": consistency(drifted),
                                "comparison": compare(drifted, before)}
        require(not report["file_drift"]["comparison"]["equal_and_consistent_observation"], "文件漂移仍错误接受旧快照")
        # native rules 是否滞后是要记录的观察，不能把平台调度差异改写成假失败/通过。
        report["file_drift"]["atomic_snapshot_proven"] = False
    finally:
        sourced.close()
    report["source_anchors"] = {"configured_files": {"userSettings": str(user_file), "projectSettings": str(project_file),
                                                       "localSettings": str(local_file)},
                                 "original_cwd": str(project), "cli_additional_directory": str(additional),
                                 "native_canonical_source_paths_returned": False,
                                 "relative_rule_authorization_executed": False}
    require(not (root / "claude/.credentials.json").exists(), "无凭据探针意外出现凭据文件")
    require(verify_binary(executable, current_platform()) == report["binary"], "原生二进制在探测中变化")
    report["binary_unchanged_after_probe"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[2]
    require(args.executable.is_absolute() and args.output.is_absolute(), "路径必须绝对")
    executable = args.executable.resolve(strict=True)
    regular_file(args.executable)
    output = args.output.resolve()
    require(not args.output.is_symlink() and not output.is_relative_to(repository) and output != executable,
            "报告必须位于源码外且不能覆盖原生文件或链接")
    require(not output.exists(), "不覆盖既有验证记录")
    output.parent.mkdir(parents=True, exist_ok=True)
    report = {"passed": False, "expected_version": VERSION, "host_os": sys.platform,
              "credentials_provided": False, "model_commands_sent": 0, "tool_execution_tested": False,
              "production_rust_adapter_verified": False, "dispatch_authorized": False,
              "atomic_permission_ceiling_proven": False,
              "session_grant_revoke": {"tested": False, "direct_control_endpoint_verified": False,
                                       "reason": "固定接口审计仅确认 can_use_tool 待审批响应可带 updatedPermissions；本轮没有真实待审批，不伪造响应"},
              "managed_only_true_tested": False, "runtime_sandbox_enforcement_tested": False,
              "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    try:
        with tempfile.TemporaryDirectory(prefix="infinishell-claude-permission-snapshot-", dir=os.environ.get("RUNNER_TEMP")) as name:
            root = Path(name).resolve()
            require(not root.is_relative_to(repository), "隔离目录必须位于源码外")
            try:
                run(executable, root, report)
                report["passed"] = True
            finally:
                report = scrub(report, root)
    except Exception as error:
        # 未知原生正文、完整 settings 和异常对象均不写入报告。
        report["failure"] = {"type": type(error).__name__, "message": str(error) if isinstance(error, ValueError) else "探针本地异常；原始对象已省略"}
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps({"permission_observation_probe_passed": report["passed"], "evidence": str(output)}, ensure_ascii=False))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
