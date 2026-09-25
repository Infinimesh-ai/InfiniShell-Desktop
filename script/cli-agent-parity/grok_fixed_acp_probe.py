#!/usr/bin/env python3
"""无凭据验证固定 Grok ACP 与自持清理；五秒 EOF 单独记录，不冒充产品监督退出。"""

import argparse
import json
import mmap
import os
from pathlib import Path
import platform
import queue
import re
import subprocess
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
from prepare_grok_cli import (LEGACY_VERSION, P0_VERSION, VERSION, VERSION_RELEASES, digest,
                              isolated_environment, regular_file, require,
                              verify_binary, verify_version)
from probe_claude_no_credentials import EOF_TIMEOUT, Recorder, repository_identity


REQUEST_TIMEOUT = 20
LEADER_TIMEOUT = 10
EXIT_TIMEOUT = 5
MISSING_SESSION = "00000000-0000-4000-8000-000000000000"
METHODS = ("initialize", "session/new", "session/cancel", "session/load", "session/resume")
SETUP_METHOD = "_x.ai/session/setup"
# 无凭据 new 在两个固定版本均完成这七个阶段；不能套用已认证的 6/9/11 阶段合同。
SETUP_VERSIONS = ("1.0.40", "1.0.41")
SETUP_PHASES = ("auth", "resolve_workspace", "folder_trust", "plugin_registry", "mcp_merge",
                "persistence_init", "spawn_session_actor")
MACOS_BINARY_BYTES = 141869568
MACOS_BINARY_SHA256 = "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb"


class EmptyLeaderPid(ValueError):
    """原生先创建并加锁，再写 PID；仅启动等待阶段允许此短暂空文件。"""


def fixed_binary(executable, version=VERSION):
    require(version in VERSION_RELEASES, "ACP 探针版本不在固定输入清单")
    machine = platform.machine().lower()
    if sys.platform == "darwin" and machine in ("arm64", "aarch64"):
        if version == LEGACY_VERSION:
            require(regular_file(executable).st_size == MACOS_BINARY_BYTES and digest(executable) == MACOS_BINARY_SHA256,
                    "本机 Grok 不匹配已有 macOS 1.0.30 完整摘要")
            return {"platform": "darwin-arm64", "bytes": MACOS_BINARY_BYTES,
                    "sha256": MACOS_BINARY_SHA256}
        return verify_binary(executable, "darwin-arm64", version)
    require(machine in ("amd64", "x86_64"), "ACP 探针没有当前架构的固定输入")
    target = f"{sys.platform}-x64"
    require(target in ("linux-x64", "win32-x64"), "ACP 探针没有当前平台的固定输入")
    return verify_binary(executable, target, version)


def requests(project, nonce):
    common = {"cwd": str(project), "mcpServers": []}
    return [
        {"jsonrpc": "2.0", "id": f"{nonce}:initialize", "method": "initialize", "params": {
            "protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}}},
        {"jsonrpc": "2.0", "id": f"{nonce}:new", "method": "session/new", "params": common},
        {"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": MISSING_SESSION}},
        {"jsonrpc": "2.0", "id": f"{nonce}:load", "method": "session/load", "params": {**common, "sessionId": MISSING_SESSION}},
        {"jsonrpc": "2.0", "id": f"{nonce}:resume", "method": "session/resume", "params": {**common, "sessionId": MISSING_SESSION}},
    ]


def valid_session_id(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except ValueError:
        return False


def validate_notification(message, version=VERSION):
    if message.get("method") == SETUP_METHOD:
        require(version in SETUP_VERSIONS and set(message) == {"jsonrpc", "method", "params"}
                and message["jsonrpc"] == "2.0", "setup 通知只允许当前固定版本的精确外层")
        params = message["params"]
        require(isinstance(params, dict) and set(params) == {"method", "phase", "sessionId"}
                and params["method"] == "session/new" and params["phase"] in SETUP_PHASES,
                "setup 通知方法、阶段或字段改变")
        if params["phase"] in SETUP_PHASES[:5]:
            require(params["sessionId"] is None, "setup 前置阶段不得提前声明会话身份")
        else:
            require(valid_session_id(params["sessionId"]), "setup 创建阶段缺少有效会话身份")
        return
    require(message == {"jsonrpc": "2.0", "method": "_x.ai/mcp/servers_updated", "params": {"mcpServers": []}},
            "无模型探针收到未经验证的通知、工具请求或模型事件")


def validate_response(request, response, version=VERSION):
    require(version in VERSION_RELEASES, "ACP 响应版本不在固定输入清单")
    require(isinstance(response, dict) and response.get("jsonrpc") == "2.0"
            and type(response.get("id")) is str and response["id"] == request["id"]
            and "method" not in response, "ACP 响应没有关联本次 method/id")
    method = request["method"]
    if method == "initialize":
        require("error" not in response and isinstance(response.get("result"), dict), "ACP initialize 没有成功")
        value = response["result"]
        metadata = value.get("_meta")
        require(type(value.get("protocolVersion")) is int and value["protocolVersion"] == 1
                and isinstance(metadata, dict) and metadata.get("agentVersion") == version,
                "ACP 版本或原生 Grok 身份不匹配")
        methods = value.get("authMethods")
        require(isinstance(methods, list) and all(isinstance(item, dict) for item in methods)
                and [item.get("id") for item in methods] == ["grok.com"]
                and metadata.get("defaultAuthMethodId") is None,
                "无凭据握手意外出现已登录认证方式")
        require(metadata.get("mcpServers") == [], "隔离握手意外加载 MCP 服务")
        return {"status": "passed", "protocol_version": 1, "agent_version": version,
                "auth_method_ids": ["grok.com"], "reported_capabilities": value.get("agentCapabilities")}
    error = response.get("error")
    require("result" not in response and isinstance(error, dict), "无凭据或缺失会话操作意外成功")
    if method == "session/new":
        require(error.get("code") == -32000 and error.get("message") == "Authentication required",
                "session/new 未明确拒绝缺失认证")
        return {"status": "blocked_by_auth", "error": error, "session_created": False}
    require(method in ("session/load", "session/resume") and error.get("code") == -32603
            and isinstance(error.get("data"), dict) and error["data"].get("code") == "FS_NOT_FOUND",
            "缺失历史没有返回可核对的 FS_NOT_FOUND")
    return {"status": "missing_session_rejected", "error": error, "history_restored": False}


def validate_transcript(records, expected, version=VERSION):
    sent = [row["message"] for row in records if row["direction"] == "stdin"]
    require(sent == expected and tuple(item["method"] for item in sent) == METHODS,
            "探针请求发生变化，或出现认证、模型输入及额外操作")
    by_id = {request["id"]: request for request in expected if "id" in request}
    observed = set()
    dispatched = set()
    setup = []
    for row in records:
        if row["direction"] == "stdin" and "id" in row["message"]:
            dispatched.add(row["message"]["id"])
        if row["direction"] != "stdout":
            continue
        message = row["message"]
        require(isinstance(message, dict), "ACP stdout 出现非 JSON 对象")
        if "id" not in message:
            validate_notification(message, version)
            if message.get("method") == SETUP_METHOD:
                setup.append(message["params"])
            continue
        identifier = message["id"]
        require(type(identifier) is str and identifier in dispatched and identifier not in observed,
                "ACP 收到重复、过期或未知请求的响应")
        validate_response(by_id[identifier], message, version)
        observed.add(identifier)
    require(observed == set(by_id), "ACP 并非每个请求都有唯一响应")
    if version in SETUP_VERSIONS:
        require(tuple(item["phase"] for item in setup) == SETUP_PHASES,
                "当前版本 setup 阶段缺失、重复或乱序")
        require(all(item["sessionId"] is None for item in setup[:5])
                and setup[5]["sessionId"] == setup[6]["sessionId"],
                "当前版本 setup 会话身份未在创建阶段稳定关联")
    else:
        require(not setup, "历史固定版本意外出现当前版本 setup 通知")


def clean(value, root):
    if isinstance(value, dict):
        return {key: "<redacted>" if re.search(r"token$|api.?key|authorization|email|secret|^hostname$|^agentId$|^agentInstanceId$", key, re.IGNORECASE)
                else clean(item, root) for key, item in value.items()}
    if isinstance(value, list):
        return [clean(item, root) for item in value]
    if isinstance(value, str):
        return value.replace(str(root), "<isolated-probe>")
    return value


def read_leader_pid(lock, *, mapped=None):
    mapped = os.name == "nt" if mapped is None else mapped
    try:
        regular_file(lock)
        with lock.open("rb", buffering=0) as handle:
            size = os.fstat(handle.fileno()).st_size
            if size == 0:
                raise EmptyLeaderPid("私有 leader PID 尚未写入")
            require(size <= 32, "私有 leader PID 文件超过诊断上限")
            if mapped:
                # Windows 的 LockFileEx 排他范围禁止普通读；只读映射不解锁、不改权限或内容。
                with mmap.mmap(handle.fileno(), size, access=mmap.ACCESS_READ) as view:
                    value = view[:]
            else:
                value = handle.read(size + 1)
            require(len(value) == size, "读取期间私有 leader PID 文件发生变化")
        text = value.decode("ascii").strip()
        require(re.fullmatch(r"[1-9][0-9]{0,9}", text) is not None and int(text) <= 0xFFFFFFFF,
                "私有 leader PID 文件不是有效原生进程身份")
        return int(text)
    except OSError as error:
        # 只记录自有临时文件的操作类别；由报告统一脱敏路径，保留原始 errno/winerror。
        error.probe_operation = "leader_lock_pid_read"
        error.probe_file_category = "private_leader_lock"
        error.probe_read_mode = "read_only_mmap" if mapped else "regular_read"
        raise


def owned_leader(leader, endpoint):
    require(leader.process.poll() is None, "自持 leader 已退出，禁止继续连接或接受其他 leader")
    require(read_leader_pid(endpoint.with_suffix(".lock")) == leader.process.pid,
            "私有 leader 锁不是本探针实际持有的进程")
    require(leader.process.poll() is None, "读取私有身份期间自持 leader 已退出")


def wait_leader(leader, endpoint):
    deadline = time.monotonic() + LEADER_TIMEOUT
    while time.monotonic() < deadline:
        require(leader.process.poll() is None, "leader 在建立私有连接前退出")
        try:
            owned_leader(leader, endpoint)
        except (EmptyLeaderPid, FileNotFoundError):
            pass
        else:
            if os.name == "nt" or endpoint.exists():
                return
        time.sleep(0.05)
    raise ValueError("私有 leader 没有在期限内建立本进程的锁与端点")


def exchange(client, leader, endpoint, request, version=VERSION):
    require(request.get("method") in METHODS, "拒绝认证、模型输入或额外 ACP 操作")
    if request["method"] == "session/cancel":
        require(request == {"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": MISSING_SESSION}},
                "取消通知只能指向固定的缺失会话")
    owned_leader(leader, endpoint)
    with client.lock:
        client.records.append({"direction": "stdin", "message": request})
    client.process.stdin.write(json.dumps(request) + "\n")
    client.process.stdin.flush()
    if "id" not in request:
        return {"status": "notification_sent_without_ack", "session_exists": False,
                "active_turn_exists": False, "running_turn_cancelled": False}
    deadline = time.monotonic() + REQUEST_TIMEOUT
    while time.monotonic() < deadline:
        owned_leader(leader, endpoint)
        require(not client.reader_errors and not leader.reader_errors, "原生输出读取失败")
        try:
            message = client.messages.get(timeout=0.1)
        except queue.Empty:
            require(client.process.poll() is None, "stdio 在响应前退出")
            continue
        require(isinstance(message, dict), "ACP stdout 必须为 JSON 对象")
        if "id" not in message:
            validate_notification(message, version)
            continue
        return validate_response(request, message, version)
    raise ValueError(f"ACP {request['method']} / {request['id']} 响应超时")


def wait_idle_exit(recorder):
    started = time.monotonic()
    try:
        recorder.process.wait(timeout=EXIT_TIMEOUT)
    except subprocess.TimeoutExpired:
        pass
    return {"exited_within_5s": recorder.process.poll() is not None,
            "exit_code_before_cleanup": recorder.process.returncode,
            "elapsed_ms": round((time.monotonic() - started) * 1000)}


def validate_stdio_completion(eof, cleanup):
    # 原生五秒 EOF 是观测能力，不是应用的监督清理合同；迟到的自然退出不能改记为五秒通过。
    require(eof.get("alive_before_stdin_eof") is True
            and type(eof.get("stdin_eof_exited_within_5s")) is bool,
            "stdio 缺少真实 EOF 观测")
    timely = eof["stdin_eof_exited_within_5s"]
    initial_exit = eof.get("exit_code_before_cleanup")
    require((timely and type(initial_exit) is int and initial_exit == 0)
            or (not timely and initial_exit is None), "stdio EOF 快照与原生退出状态不一致")
    require(cleanup.get("forced_termination") is False
            and cleanup.get("owned_process_exited") is True
            and type(cleanup.get("exit_code")) is int and cleanup["exit_code"] == 0
            and "error" not in cleanup, "stdio 没有在自持清理前自然正常退出")
    return {"observation_timeout_seconds": EOF_TIMEOUT,
            "exited_within_observation": timely, "delayed_natural_exit": not timely,
            "natural_exit_before_owned_cleanup": True, "product_supervisor_verified": False}


def run(executable, root, report, version=VERSION):
    env = isolated_environment(root)
    report.update(repository_identity(env))
    report["binary"] = fixed_binary(executable, version)
    report["cli_version"] = verify_version(executable, root, version)
    project = root / "project"
    project.mkdir()
    endpoint = root / "leader.sock"
    require(os.name == "nt" or len(os.fsencode(endpoint)) < 100, "Unix 私有 socket 路径超过保守上限")
    env["GROK_LEADER_SOCKET"] = str(endpoint)
    report["endpoint"] = {"kind": "hashed_windows_named_pipe" if os.name == "nt" else "unix_socket",
                          "path": "<isolated-probe>/leader.sock", "inherited": False}
    leader = Recorder([str(executable), "agent", "leader", "--relay-on-demand", "--no-auto-update",
                       "--leader-socket", str(endpoint)], env, project)
    leader.process.stdin.close()
    report["leader_pid"] = leader.process.pid
    client = None
    expected = requests(project, str(uuid.uuid4()))
    cleanup = {}
    try:
        report["phase"] = "wait_for_private_leader"
        wait_leader(leader, endpoint)
        report["leader_identity_read_mode"] = "read_only_mmap" if os.name == "nt" else "regular_read"
        report["phase"] = "start_private_stdio"
        client = Recorder([str(executable), "agent", "stdio", "--leader-socket", str(endpoint)], env, project)
        report["stdio_pid"] = client.process.pid
        for request in expected:
            report["phase"] = request["method"]
            result = exchange(client, leader, endpoint, request, version)
            report["cases"].append({"method": request["method"], "request_id": request.get("id"), **result})
        owned_leader(leader, endpoint)
        report["private_leader_pid_confirmed"] = True
        report["phase"] = "stdio_eof_and_owned_cleanup"
        report["stdio_eof"] = client.finish_eof()
        report["leader_after_stdio_eof"] = wait_idle_exit(leader)
    finally:
        # 分别关闭两个自持句柄；强制清理与原生 EOF 自行退出必须分开记录。
        for name, recorder in (("stdio", client), ("leader", leader)):
            if recorder is None:
                continue
            try:
                forced = recorder.close()
                cleanup[name] = {"forced_termination": forced, "exit_code": recorder.process.returncode,
                                 "owned_process_exited": recorder.process.poll() is not None}
            except Exception as error:
                cleanup[name] = {"error": str(error), "exit_code": recorder.process.poll(),
                                 "owned_process_exited": recorder.process.poll() is not None}
            finally:
                report[f"{name}_events"] = clean(recorder.records, root)
        report["cleanup"] = cleanup
        lock = endpoint.with_suffix(".lock")
        if lock.exists():
            require(read_leader_pid(lock) == leader.process.pid,
                    "收尾检测到替换 leader；不能宣称私有连接已清理")
        report["endpoint_file_remaining"] = endpoint.exists()
        report["leader_lock_file_remaining"] = lock.exists()
    require(client is not None, "stdio 没有启动")
    validate_transcript(client.records, expected, version)
    require(all(item.get("owned_process_exited") and "error" not in item for item in cleanup.values()),
            "自持原生进程或输出读取线程未完整清理")
    # 保持既有两次观察的时限，不新增等待；产品退出另由真实监督器四场景门禁核对。
    report["native_stdio_eof_observation"] = validate_stdio_completion(report["stdio_eof"], cleanup["stdio"])
    require(not report["leader_after_stdio_eof"]["exited_within_5s"] or
            report["leader_after_stdio_eof"]["exit_code_before_cleanup"] == 0,
            "leader 在收尾前异常退出")
    require(not (root / "grok/auth.json").exists(), "无凭据探针意外产生 auth.json")
    require(fixed_binary(executable, version) == report["binary"], "原生文件在探测期间发生变化")
    report["binary_unchanged"] = True
    report["phase"] = "complete"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", choices=VERSION_RELEASES, default=VERSION,
                        help=f"固定原生版本，默认当前正式版 {VERSION}")
    args = parser.parse_args()
    require(args.executable.is_absolute() and args.output.is_absolute(), "可执行文件和证据必须使用绝对路径")
    regular_file(args.executable)
    executable = args.executable.resolve(strict=True)
    require(not args.output.exists() and not args.output.is_symlink(), "证据文件必须是新路径，不能覆盖旧验证")
    output = args.output.resolve()
    require(not output.is_relative_to(Path(__file__).resolve().parents[2]), "证据必须位于源树外")
    output.parent.mkdir(parents=True, exist_ok=True)
    report = {"passed": False, "scope": "unauthenticated_fixed_acp_boundaries", "cases": [],
              "acceptance_contract": "fixed_acp_owned_cleanup_with_native_eof_observation",
              "credentials_provided": False, "authenticate_sent": False, "model_input_submitted": False,
              "running_approval_verified": False, "running_cancel_verified": False, "history_recovery_verified": False,
              "model_http_traffic_measured": False, "requested_cli_version": args.version}
    # Unix 用短路径避免 sockaddr_un 上限；Windows 由原生实现把唯一私有路径映射为命名管道。
    temporary_parent = None if os.name == "nt" else "/tmp"
    root = None
    try:
        with tempfile.TemporaryDirectory(prefix="grok-acp-", dir=temporary_parent) as temporary:
            root = Path(temporary).resolve()
            run(executable, root, report, args.version)
        require(not root.exists(), "探针私有配置及端点目录未完成清理")
        report["passed"] = True
    except Exception as error:
        report["failure"] = {"type": type(error).__name__, "message": clean(str(error), root)}
        for field in ("errno", "winerror", "probe_operation", "probe_file_category", "probe_read_mode"):
            if getattr(error, field, None) is not None:
                report["failure"][field] = getattr(error, field)
    finally:
        report["private_directory_removed"] = root is not None and not root.exists()
        with output.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(clean(report, root), target, ensure_ascii=True, indent=2)
            target.write("\n")
    require(report["passed"], "Grok ACP 边界未通过，详见独立证据")
    print(output)


if __name__ == "__main__":
    main()
