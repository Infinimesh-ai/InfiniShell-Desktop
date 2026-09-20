#!/usr/bin/env python3
"""无认证、零模型核对固定 Claude Windows 路径与正式 SessionStart 的 ConPTY 字节。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unicodedata
import uuid

sys.dont_write_bytecode = True
from prepare_claude_cli import isolated_environment, regular_file, verify_binary, verify_version
import apply_notification_patch as patch
import probe_codex_windows_conpty as conpty_host
from probe_codex_windows_conpty import MAX_OUTPUT, WinApi, notifications, run_conpty
from probe_codex_plugin_cache_refresh import WindowsProbeJob, suspended_creation
from codex_windows_hook_command import windows_environment


SCHEMA = 1
MARKER = b"isolated Claude Windows notification verification\n"
OUTER_TIMEOUT = 420
CASE_TIMEOUT = 45
CASE_NAMES = ("插件 空 格", "插件 ' $(touch INJECTED) `touch INJECTED` & %PATH% !name! ^ ()")
SCRIPT_NAMES = (
    "probe_claude_windows_notifications.py", "prepare_claude_cli.py", "apply_notification_patch.py",
    "probe_codex_windows_conpty.py", "probe_codex_plugin_cache_refresh.py", "probe_codex_windows_hooks.py",
    "codex_windows_hook_command.py", "codex_windows_hook_inputs.py", "codex_windows_formal.py",
    "codex_persistent_source.py", "codex_windows_notify.py",
)


class ProbeFailure(ValueError):
    """只携带此探针写死的诊断，不记录原生错误原文。"""


def require(condition, message):
    if not condition:
        raise ProbeFailure(message)


def digest(path):
    regular_file(path)
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path, value, *, exclusive=False):
    data = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    if exclusive:
        with path.open("xb") as output:
            output.write(data)
        return
    # 回执更新原子替换，外层截止中断时保留上一份完整证据。
    descriptor, name = tempfile.mkstemp(prefix=".receipt-", dir=path.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def bounded_json(path):
    require(regular_file(path).st_size <= 1024 * 1024, "证据文件超过上限")
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), "证据必须是对象")
    return value


def failure(error):
    # 原生输出、配置和异常参数不直接归档，避免把机器策略文本或环境内容带出私有目录。
    record = {"type": type(error).__name__, "message_sha256": hashlib.sha256(str(error).encode()).hexdigest()}
    if isinstance(error, ProbeFailure):
        record["reason"] = str(error)
    return record


def sanitize_trace(record):
    # 公共 Job 帮助器的原始 Win32 异常只留摘要；不上传外部路径或参数。
    for key in ("startup_failure", "startup_cleanup_failure", "handle_close_failures"):
        if key in record:
            raw = json.dumps(record.pop(key), ensure_ascii=False, sort_keys=True).encode()
            record[key + "_sha256"] = hashlib.sha256(raw).hexdigest()


def source_hashes(repo):
    files = [repo / "script/cli-agent-parity" / name for name in SCRIPT_NAMES]
    files += [repo / "specs/cli-agent-parity/fixtures/claude-warp-compatible-original-trees.json"]
    files += sorted(path for path in (repo / "app/assets/bundled/cli-agent-plugins/claude").rglob("*") if path.is_file())
    return {path.relative_to(repo).as_posix(): digest(path) for path in files}


def identity(repo, env):
    commit = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], env=env,
                            capture_output=True, text=True, timeout=10, check=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", str(repo), "status", "--porcelain"], env=env,
                           capture_output=True, timeout=10, check=True).stdout
    require(len(commit) == 40 and all(c in "0123456789abcdef" for c in commit) and not dirty,
            "必须在干净的同提交检出中执行")
    return commit


def select_dependencies(bash, jq):
    if bash is None:
        candidates = [Path(value) / suffix for key, suffix in (("PROGRAMFILES", "Git/usr/bin/bash.exe"),
                      ("LOCALAPPDATA", "Programs/Git/usr/bin/bash.exe")) if (value := os.environ.get(key))]
        bash = next((path for path in candidates if path.is_file()), None)
    if jq is None:
        found = shutil.which("jq.exe")
        jq = Path(found) if found else None
    require(bash is not None and bash.is_absolute() and jq is not None and jq.is_absolute(), "缺少原生 Git Bash/jq")
    require(bash.name.lower() == "bash.exe" and (bash.parent / "msys-2.0.dll").is_file(), "拒绝 WSL 或未知 Bash")
    require(jq.name.lower() == "jq.exe", "只接受原生 jq.exe")
    regular_file(bash); regular_file(jq)
    return bash.resolve(strict=True), jq.resolve(strict=True)


def case_environment(root, dependencies, executable):
    env = isolated_environment(root)
    # 此处要验证真正交互模式，不能继承隔离 SDK 探针的入口标记或任何宿主认证变量。
    env.pop("CLAUDE_CODE_ENTRYPOINT", None)
    for key in ("PATH", "COMSPEC", "SYSTEMROOT", "WINDIR"):
        if key in dependencies:
            env[key] = dependencies[key]
    env["PATH"] = str(executable.parent) + os.pathsep + dependencies["PATH"]
    env.update(WARP_CLI_AGENT_PROTOCOL_VERSION="1", WARP_CLIENT_VERSION="infinishell-windows-hook-probe",
               TERM_PROGRAM="WarpTerminal", TERM="xterm-256color", GIT_CONFIG_NOSYSTEM="1",
               GIT_CONFIG_GLOBAL=str(root / "empty-gitconfig"), GIT_TERMINAL_PROMPT="0")
    (root / "empty-gitconfig").touch()
    return env


def seed_completed_onboarding(root, project):
    # 固定 2.1.273 的 Zyn/FB/lr/C9e：私有配置目录、NFC 与 Windows 正斜杠项目键。
    # 只信任本轮自建空目录；不增加账号、授权工具、危险模式或用户真实配置。
    key = unicodedata.normalize("NFC", project.resolve(strict=True).as_posix())
    value = {"hasCompletedOnboarding": True, "theme": "dark", "autoUpdates": False,
             "projects": {key: {"hasTrustDialogAccepted": True}}}
    write_json(root / "claude/.claude.json", value, exclusive=True)
    return value


def native_command(executable, plugin, *, session_id=None):
    arguments = [str(executable)]
    if session_id is None:
        arguments.append("--init-only")
    else:
        require(str(uuid.UUID(session_id)) == session_id, "会话 ID 必须是本轮 UUID")
        arguments.extend(("--session-id", session_id))
    arguments.extend(("--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
                      "--plugin-dir", str(plugin)))
    return arguments


def prepare_plugin(repo, destination, staging_parent):
    metadata, replacements = patch.bundle_data(repo / "app/assets/bundled/cli-agent-plugins", "claude")
    originals = json.loads((repo / "specs/cli-agent-parity/fixtures/claude-warp-compatible-original-trees.json").read_text(encoding="utf-8"))["2.2.0"]
    expected = next(base["tree_sha256"] for base in metadata["compatible_bases"] if base["version"] == "2.2.0")
    require(set(originals) == set(expected), "固定原树文件集合不符")
    destination.mkdir(parents=True, exist_ok=False)
    for name, contents in originals.items():
        require(not Path(name).is_absolute() and all(part not in (".", "..") for part in Path(name).parts), "原树路径越界")
        data = contents.encode("utf-8")
        require(hashlib.sha256(data).hexdigest() == expected[name], "固定原树摘要不符")
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        target.chmod(0o755 if name.endswith(".sh") else 0o644)
    patch.validate_tree(destination, "2.2.0", metadata)
    patch.apply_files(destination, metadata, replacements, staging_parent=staging_parent)
    patch.validate_tree(destination, "2.2.0", metadata)
    return {name: digest(destination / name) for name in expected}


def verify_plugin(root, expected):
    observed = {path.relative_to(root).as_posix(): digest(path) for path in root.rglob("*") if path.is_file()}
    require(observed == expected and not any(path.is_symlink() for path in root.rglob("*")), "正式插件树发生改变")


def credential_boundary(root):
    # 只查本轮创建的目录，不打开任何账号文件。
    return not any(path.name in (".credentials.json", "auth.json", "credentials.json") for path in root.rglob("*"))


def raw_bytes(path):
    if not path.exists():
        return b""
    require(regular_file(path).st_size <= MAX_OUTPUT, "ConPTY 输出超过固定上限")
    return path.read_bytes()


def notification_projection(value, expected):
    require(isinstance(expected, dict) and isinstance(expected.get("session_id"), str)
            and isinstance(expected.get("cwd"), str), "关联条件不完整")
    try:
        fresh_id = str(uuid.UUID(expected["session_id"]))
    except ValueError:
        raise ProbeFailure("关联 UUID 格式不符") from None
    require(fresh_id == expected["session_id"], "关联 UUID 格式不符")
    require(isinstance(value, dict) and type(value.get("v")) is int and value["v"] == 1
            and value.get("agent") == "claude" and value.get("event") == "session_start"
            and value.get("session_id") == expected["session_id"] and value.get("cwd") == expected["cwd"]
            and value.get("plugin_version") == "2.2.0", "原生通知关联不符")
    return {key: value[key] for key in ("v", "agent", "event", "session_id", "cwd", "plugin_version")}


def transport_match(raw, expected):
    values = notifications(raw)
    # 唯一载荷必须来自未改动的正式脚本；诊断 OSC、自造 marker 与其他会话不能满足此条件。
    require(len(values) == 1, "缺失、重复或额外通知")
    return notification_projection(values[0], expected)


def cleanup_process(process, job, record):
    errors = []
    try:
        if process is not None:
            record["exit_code_before_cleanup"] = process.poll()
        if job.active() != 0:
            record["forced_cleanup"] = True
            job.terminate()
        record["job_empty"] = job.wait_empty(5) == 0
        if process is not None:
            process.wait(timeout=5)
    except Exception as error:
        errors.append(failure(error))
    finally:
        try:
            job.close()
        except Exception as error:
            errors.append(failure(error))
    sanitize_trace(record)
    record["cleanup_errors"] = errors
    require(record.get("job_empty") is True and not errors, "私有进程树清理未确认")


def run_path_case(repo, root, executable, dependencies, name):
    import _winapi
    record = {"phase": "init_only_path", "name": name, "passed": False, "scripts_instrumented": True,
              "terminal_transport_verified": False, "forced_cleanup": False}
    root.mkdir()
    project, plugin = root / "project", root / name
    project.mkdir()
    env = case_environment(root, dependencies, executable)
    for child in (".claude-plugin", "hooks", "scripts"):
        (plugin / child).mkdir(parents=True)
    hooks = json.loads((repo / "app/assets/bundled/cli-agent-plugins/claude/hooks/hooks.json").read_text(encoding="utf-8"))
    write_json(plugin / ".claude-plugin/plugin.json", {"name": "infinishell-path-probe", "version": "0.1.0"})
    write_json(plugin / "hooks/hooks.json", {"hooks": {"SessionStart": hooks["hooks"]["SessionStart"]}})
    (plugin / "scripts/on-session-start.sh").write_text(
        '#!/bin/bash\njq -c --arg root "$CLAUDE_PLUGIN_ROOT" '
        "'{hook_event_name, source, plugin_root:$root}' > \"$INFINISHELL_NOTIFICATION_PATH_PROBE/marker.json\"\n",
        encoding="utf-8", newline="\n")
    env["INFINISHELL_NOTIFICATION_PATH_PROBE"] = root.as_posix()
    command = native_command(executable, plugin)
    job, process = WindowsProbeJob(), None
    try:
        with suspended_creation(_winapi, job, command, env, project, record):
            process = subprocess.Popen(command, env=env, cwd=project, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        stdout, stderr = process.communicate(timeout=20)
        require(len(stdout) <= MAX_OUTPUT and len(stderr) <= MAX_OUTPUT, "初始化输出超过固定上限")
        record.update(exit_code=process.returncode, stdout_sha256=hashlib.sha256(stdout).hexdigest(),
                      stderr_sha256=hashlib.sha256(stderr).hexdigest())
        marker = bounded_json(root / "marker.json")
        record.update(native_session_start=marker.get("hook_event_name") == "SessionStart" and marker.get("source") == "startup",
                      plugin_root_preserved=marker.get("plugin_root") == str(plugin),
                      injection_marker_absent=not any(root.rglob("INJECTED")))
        require(process.returncode == 0 and record["native_session_start"] and record["plugin_root_preserved"]
                and record["injection_marker_absent"], "原生路径参数验证失败")
        record["passed"] = True
    except Exception as error:
        record["failure"] = failure(error)
    finally:
        try:
            cleanup_process(process, job, record)
        except Exception as error:
            record["cleanup_failure"] = failure(error)
        if process is not None:
            for pipe in (process.stdout, process.stderr):
                if pipe is not None:
                    pipe.close()
        record["credential_boundary"] = credential_boundary(root)
        record["passed"] = record["passed"] and record.get("job_empty") is True and not record["forced_cleanup"] and not record.get("cleanup_failure") and record["credential_boundary"]
    return record


def send_exit_key(api):
    import ctypes
    from ctypes import wintypes
    class Key(ctypes.Structure):
        _fields_ = [("down", wintypes.BOOL), ("repeat", wintypes.WORD), ("virtual", wintypes.WORD),
                    ("scan", wintypes.WORD), ("character", wintypes.WCHAR), ("state", wintypes.DWORD)]
    class Event(ctypes.Union):
        _fields_ = [("key", Key), ("padding", ctypes.c_byte * 16)]
    class Record(ctypes.Structure):
        _fields_ = [("kind", wintypes.WORD), ("event", Event)]
    write = api.kernel.WriteConsoleInputW
    write.argtypes, write.restype = [wintypes.HANDLE, ctypes.POINTER(Record), wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL
    handle = api.kernel.CreateFileW("CONIN$", 0xC0000000, 3, None, 3, 0, None)
    require(handle not in (None, ctypes.c_void_p(-1).value), "私有 ConPTY 输入不可用")
    try:
        events = (Record * 2)()
        for index, down in enumerate((True, False)):
            events[index].kind = 1
            events[index].event.key = Key(down, 1, 0x43, 0, "\x03", 8)
        count = wintypes.DWORD()
        api.check(write(handle, events, 2, ctypes.byref(count)))
        require(count.value == 2, "退出按键写入不完整")
    finally:
        api.kernel.CloseHandle(handle)


def private_path(root, value, *, exists=False):
    require(isinstance(value, str) and Path(value).is_absolute(), "私有路径格式不符")
    path = Path(value)
    require(path != root and path.is_relative_to(root) and path.resolve(strict=exists).is_relative_to(root), "私有路径越界")
    current = path
    while current != root:
        if current.exists() or current.is_symlink():
            require(not current.is_symlink() and not getattr(current.lstat(), "st_file_attributes", 0) & 0x400,
                    "私有路径含符号链接或重解析点")
        current = current.parent
    return path


def owned_root(root):
    require(root.is_absolute() and root.resolve(strict=True) == root and not root.is_symlink()
            and not getattr(root.lstat(), "st_file_attributes", 0) & 0x400, "私有目录所有权不符")
    marker = root / ".probe-owned"
    regular_file(marker)
    require(marker.read_bytes() == MARKER, "私有目录所有权标记改变")


def load_configuration(path):
    value = bounded_json(path)
    require(type(value.get("schema")) is int and value["schema"] == SCHEMA
            and isinstance(value.get("private_root"), str), "私有配置格式不符")
    root = Path(value["private_root"])
    owned_root(root)
    private_path(root, str(path), exists=True)
    for key in ("worker_report", "case_root", "project", "plugin", "raw_output", "driver_report"):
        if key in value:
            private_path(root, value[key])
    return value


def run_driver(configuration):
    import _winapi
    config = load_configuration(configuration)
    root, project, plugin = (Path(config[key]) for key in ("case_root", "project", "plugin"))
    report = {"phase": "interactive_conpty", "passed": False, "scripts_instrumented": False,
              "fixture_completed_onboarding": True, "fixture_project_trust": True,
              "default_first_run_verified": False, "model_inputs_sent": 0, "forced_cleanup": False,
              "native_child_attached": False, "native_session_start_received": False}
    job, process = WindowsProbeJob(), None
    try:
        executable = Path(config["executable"])
        verify_binary(executable, "win32-x64")
        env = case_environment(root, config["dependencies"], executable)
        seed_completed_onboarding(root, project)
        command = native_command(executable, plugin, session_id=config["session_id"])
        api = WinApi()
        with suspended_creation(_winapi, job, command, env, project, report):
            # 不重定向任何标准句柄：真正 Claude UI 必须继承此 HPCON。
            process = subprocess.Popen(command, env=env, cwd=project)
        expected = {"session_id": config["session_id"], "cwd": str(project)}
        started = time.monotonic()
        deadline = started + CASE_TIMEOUT
        while time.monotonic() < deadline and process.poll() is None:
            report["native_child_attached"] |= any(member["pid"] == process.pid for member in api.console_members())
            try:
                report["notification"] = transport_match(raw_bytes(Path(config["raw_output"])), expected)
                report["native_session_start_received"] = True
                break
            except (ValueError, json.JSONDecodeError, UnicodeError):
                time.sleep(0.05)
        report["notification_wait_seconds"] = round(time.monotonic() - started, 3)
        require(report["native_child_attached"] and report["native_session_start_received"], "前台原生通知未在期限内确认")
        # 只发送退出按键；没有任意文本、提示词、审批确认或模型输入。
        send_exit_key(api)
        time.sleep(0.3)
        if process.poll() is None:
            send_exit_key(api)
        process.wait(timeout=5)
        require(process.returncode == 0, "原生会话未正常退出")
        verify_plugin(plugin, config["plugin_hashes"])
        report["formal_resource_hashes_match"] = True
        report["passed"] = True
    except Exception as error:
        report["failure"] = failure(error)
    finally:
        try:
            cleanup_process(process, job, report)
        except Exception as error:
            report["cleanup_failure"] = failure(error)
        report["credential_boundary"] = credential_boundary(root)
        report["passed"] = report["passed"] and report.get("job_empty") is True and not report["forced_cleanup"] and not report.get("cleanup_failure") and report["credential_boundary"]
        write_json(Path(config["driver_report"]), report, exclusive=True)
    return 0 if report["passed"] else 1


def contained_driver_creation(native_create, command, cwd, trace):
    expected_flags = 0x80000 | 0x400 | 0x1000000
    invoked = False

    def create(*arguments):
        nonlocal invoked
        # 公共 ConPTY 宿主要求 breakaway，本探针必须持续继承控制器的禁止脱离 Job。
        # 只适配本轮精确 driver；共享接口变化或任何额外创建都拒绝，不作通用 Win32 拦截。
        require(not invoked and len(arguments) == 10 and arguments[0] == command[0]
                and getattr(arguments[1], "value", None) == subprocess.list2cmdline(command)
                and arguments[4] is False and arguments[5] == expected_flags and arguments[7] == str(cwd),
                "ConPTY driver 创建契约不符")
        invoked = True
        changed = list(arguments)
        changed[5] &= ~0x1000000
        trace.update(original_create_flags=arguments[5], actual_create_flags=changed[5],
                     breakaway_removed=True)
        return native_create(*changed)

    return create


def run_contained_conpty(dll_path, command, environment, cwd, output, timeout, trace):
    original_factory = conpty_host.WinApi

    class ContainedApi(original_factory):
        def __init__(self):
            super().__init__()
            self.kernel.CreateProcessW = contained_driver_creation(self.kernel.CreateProcessW, command, cwd, trace)

    # worker 只有一个串行宿主；仅此次调用替换工厂，finally 恢复，不改共享文件或其余进程。
    conpty_host.WinApi = ContainedApi
    try:
        result = run_conpty(dll_path, command, environment, cwd, output, timeout)
        require(trace.get("breakaway_removed") is True and result.get("create_flags") == trace["original_create_flags"],
                "ConPTY driver 未经预期的 Job 继承适配")
        result["create_flags"] = trace["actual_create_flags"]
        result.update(trace)
        return result
    finally:
        conpty_host.WinApi = original_factory


def run_worker(configuration):
    config = load_configuration(configuration)
    repo, root, executable = (Path(config[key]) for key in ("repo", "private_root", "executable"))
    report = {"schema": SCHEMA, "passed": False, "path_cases": [], "transport_cases": [],
              "model_inputs_sent": 0, "credentials_provided": False, "credential_files_read": False,
              "product_installer_exercised": False, "full_lifecycle_verified": False,
              "default_first_run_verified": False, "raw_pty_archived": False}
    write_json(Path(config["worker_report"]), report, exclusive=True)
    try:
        report["binary"] = verify_binary(executable, "win32-x64")
        report["version"] = verify_version(executable, root)
        dependencies = windows_environment(Path(config["bash"]), Path(config["jq"]))
        dependency_files = {"bash.exe": Path(config["bash"]), "jq.exe": Path(config["jq"]),
                            "msys-2.0.dll": Path(config["bash"]).parent / "msys-2.0.dll"}
        report["dependency_sha256"] = {name: digest(path) for name, path in dependency_files.items()}
        report["source_sha256"] = source_hashes(repo)
        host = root / "host"
        (host / "x64").mkdir(parents=True)
        for name, target in (("conpty.dll", "conpty.dll"), ("OpenConsole.exe", "x64/OpenConsole.exe")):
            source = repo / "app/assets/windows/x64" / name
            shutil.copyfile(source, host / target)
            require(digest(source) == digest(host / target), "ConPTY 资产复制不一致")
        report["conpty_assets"] = {name: digest(host / name) for name in ("conpty.dll", "x64/OpenConsole.exe")}
        for index, name in enumerate(CASE_NAMES):
            report["path_cases"].append(run_path_case(repo, root / f"path-{index}", executable, dependencies, name))
            write_json(Path(config["worker_report"]), report)
        for index, name in enumerate(CASE_NAMES):
            case = {"name": name, "passed": False, "session_id": str(uuid.uuid4())}
            report["transport_cases"].append(case)
            case_root = root / f"interactive-{index}"
            case_root.mkdir()
            project = case_root / "工作区 空格"
            project.mkdir()
            plugin = case_root / name
            hashes = prepare_plugin(repo, plugin, case_root)
            case["plugin_sha256"] = hashes
            case["expected"] = {"session_id": case["session_id"], "cwd": str(project)}
            write_json(Path(config["worker_report"]), report)
            raw_output = root / f"case-{index}.pty.bin"
            driver_report = root / f"case-{index}.driver.json"
            driver_config = {**config, "dependencies": dependencies, "case_root": str(case_root), "project": str(project),
                             "plugin": str(plugin), "plugin_hashes": hashes, "session_id": case["session_id"],
                             "raw_output": str(raw_output), "driver_report": str(driver_report)}
            path = root / f"case-{index}.config.json"
            write_json(path, driver_config, exclusive=True)
            env = case_environment(root / f"host-env-{index}", dependencies, executable)
            try:
                case["conpty"] = run_contained_conpty(host / "conpty.dll", [sys.executable, "-B", str(Path(__file__).resolve()),
                    "--driver-config", str(path)], env, root, raw_output, 90, case.setdefault("conpty_creation", {}))
            except Exception as error:
                case["conpty_failure"] = failure(error)
            finally:
                if driver_report.exists():
                    case["native"] = bounded_json(driver_report)
                raw = raw_bytes(raw_output)
                case["raw_pty"] = {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
            try:
                case["notification"] = transport_match(raw, case["expected"])
                require(case.get("native", {}).get("passed") is True and case.get("conpty", {}).get("exit_code") == 0
                        and case["conpty"].get("console_close_and_output_eof_confirmed") is True
                        and not case.get("conpty_failure"), "原生退出与ConPTY收尾未确认")
                verify_plugin(plugin, hashes)
                case["formal_resource_hashes_match"] = True
                case["passed"] = True
            except Exception as error:
                case["failure"] = failure(error)
            write_json(Path(config["worker_report"]), report)
        require(source_hashes(repo) == report["source_sha256"] and verify_binary(executable, "win32-x64") == report["binary"], "执行期间固定输入变化")
        require({name: digest(path) for name, path in dependency_files.items()} == report["dependency_sha256"],
                "执行期间原生依赖变化")
        report["inputs_unchanged"] = True
        report["passed"] = all(case["passed"] for case in report["path_cases"] + report["transport_cases"])
    except Exception as error:
        report["failure"] = failure(error)
    finally:
        write_json(Path(config["worker_report"]), report)
    return 0 if report["passed"] else 1


def validate_acceptance(value):
    require(isinstance(value, dict) and type(value.get("schema")) is int and value["schema"] == SCHEMA and value.get("passed") is True
            and value.get("inputs_unchanged") is True and type(value.get("model_inputs_sent")) is int and value["model_inputs_sent"] == 0
            and value.get("credentials_provided") is False and value.get("credential_files_read") is False
            and value.get("full_lifecycle_verified") is False and value.get("product_installer_exercised") is False and value.get("default_first_run_verified") is False,
            "总证据缺失或越过验证范围")
    for key in ("path_cases", "transport_cases"):
        rows = value.get(key)
        require(isinstance(rows, list) and len(rows) == len(CASE_NAMES), "场景数量不符")
        require(all(isinstance(row, dict) and row.get("name") == name and row.get("passed") is True
                    for row, name in zip(rows, CASE_NAMES)), "原生场景没有全部通过")
    for row in value["path_cases"]:
        require(row.get("phase") == "init_only_path" and row.get("scripts_instrumented") is True
                and row.get("terminal_transport_verified") is False and row.get("job_empty") is True
                and row.get("forced_cleanup") is False and row.get("credential_boundary") is True
                and row.get("native_session_start") is True and row.get("plugin_root_preserved") is True
                and row.get("injection_marker_absent") is True and row.get("assigned_before_resume") is True
                and row.get("cleanup_errors") == [] and "cleanup_failure" not in row
                and type(row.get("exit_code")) is int and row["exit_code"] == 0,
                "初始化路径验证不能冒充终端传输")
    ids = [row.get("session_id") for row in value["transport_cases"]]
    require(all(isinstance(item, str) for item in ids) and len(set(ids)) == len(ids), "必须使用各自的新会话 UUID")
    for row in value["transport_cases"]:
        native = row.get("native")
        require(isinstance(native, dict) and native.get("phase") == "interactive_conpty"
                and all(native.get(key) is True for key in ("passed", "fixture_completed_onboarding", "fixture_project_trust",
                        "native_child_attached", "native_session_start_received", "formal_resource_hashes_match", "job_empty", "credential_boundary"))
                and native.get("default_first_run_verified") is False and native.get("scripts_instrumented") is False
                and native.get("forced_cleanup") is False and native.get("assigned_before_resume") is True
                and native.get("cleanup_errors") == [] and "cleanup_failure" not in native
                and type(native.get("model_inputs_sent")) is int and native["model_inputs_sent"] == 0,
                "必须确认未改正式 hook 的真实前台交互通知")
        require(row.get("formal_resource_hashes_match") is True and isinstance(row.get("conpty"), dict)
                and row["conpty"].get("console_close_and_output_eof_confirmed") is True
                and type(row["conpty"].get("exit_code")) is int and row["conpty"]["exit_code"] == 0
                and row["conpty"].get("breakaway_removed") is True
                and row["conpty"].get("original_create_flags") == 0x1080400
                and row["conpty"].get("actual_create_flags") == row["conpty"].get("create_flags") == 0x80400,
                "ConPTY没有正常收尾")
        notification = notification_projection(row.get("notification"), row.get("expected"))
        require(isinstance(notification, dict) and notification.get("session_id") == row.get("session_id")
                and notification == native.get("notification"), "两侧真实通知没有关联")


def require_native_host():
    require(os.name == "nt" and sys.implementation.name == "cpython" and struct.calcsize("P") == 8,
            "需要原生 Windows x64 CPython")


def public_run(args):
    repo = Path(__file__).resolve().parents[2]
    output = args.output
    require(output is not None and output.is_absolute() and not output.is_symlink()
            and not output.resolve().is_relative_to(repo), "报告必须位于仓库之外")
    output.parent.mkdir(parents=True, exist_ok=True)
    report = {"schema": SCHEMA, "passed": False, "host_os": sys.platform, "outer_timeout_seconds": OUTER_TIMEOUT,
              "credentials_provided": False, "credential_files_read": False, "model_inputs_sent": 0,
              "fixture_completed_onboarding": True, "default_first_run_verified": False,
              "production_windows_gate_changed": False, "forced_cleanup": False}
    write_json(output, report, exclusive=True)
    root, job, process = None, None, None
    try:
        require_native_host()
        executable = args.executable
        require(executable is not None and executable.is_absolute() and executable.name.lower() == "claude.exe", "需要固定原生 Claude 路径")
        report["binary"] = verify_binary(executable, "win32-x64")
        bash, jq = select_dependencies(args.bash_executable, args.jq_executable)
        root = Path(tempfile.mkdtemp(prefix="infinishell-claude-windows-notify-", dir=os.environ.get("RUNNER_TEMP"))).resolve()
        require(not root.is_relative_to(repo), "私有目录必须位于仓库外")
        (root / ".probe-owned").write_bytes(MARKER)
        env = isolated_environment(root / "controller")
        report["source_commit"] = identity(repo, env)
        config = {"schema": SCHEMA, "repo": str(repo), "private_root": str(root), "executable": str(executable),
                  "bash": str(bash), "jq": str(jq), "worker_report": str(root / "worker.json")}
        config_path = root / "worker.config.json"
        write_json(config_path, config, exclusive=True)
        command = [sys.executable, "-B", str(Path(__file__).resolve()), "--worker-config", str(config_path)]
        import _winapi
        job = WindowsProbeJob()
        with suspended_creation(_winapi, job, command, env, root, report):
            process = subprocess.Popen(command, env=env, cwd=root, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        process.wait(timeout=OUTER_TIMEOUT)
        report["worker_exit_code"] = process.returncode
        report["worker"] = bounded_json(Path(config["worker_report"]))
        validate_acceptance(report["worker"])
        require(process.returncode == 0, "worker没有正常退出")
        report["passed"] = True
    except Exception as error:
        report["failure"] = failure(error)
    finally:
        if job is not None:
            try:
                cleanup_process(process, job, report)
            except Exception as error:
                report["cleanup_failure"] = failure(error)
        elif process is None:
            report["job_empty"] = True
        if root is not None and report.get("job_empty") is True:
            try:
                # 仅在外层 Job 已空后删除含私有配置和宿主 DLL 的自建目录。
                owned_root(root)
                # 总期限到达后仍回收已完成的安全回执，不能先删掉唯一失败证据。
                for name in ("worker.json", "case-0.driver.json", "case-1.driver.json"):
                    receipt = root / name
                    if receipt.exists():
                        try:
                            record = bounded_json(private_path(root, str(receipt), exists=True))
                            report.setdefault("recovered_receipts", {})[name] = record
                        except Exception as error:
                            report.setdefault("receipt_errors", {})[name] = failure(error)
                shutil.rmtree(root)
                report["private_directory_removed"] = True
            except Exception as error:
                report["private_directory_removed"] = False
                report["cleanup_failure"] = failure(error)
        report["passed"] = report["passed"] and report.get("job_empty") is True and not report["forced_cleanup"] and report.get("private_directory_removed") is True and not report.get("cleanup_failure")
        write_json(output, report)
    print(json.dumps({"passed": report["passed"], "evidence": str(output)}, ensure_ascii=False))
    return 0 if report["passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("executable", "bash-executable", "jq-executable", "output", "worker-config", "driver-config"):
        parser.add_argument("--" + name, type=Path, help=argparse.SUPPRESS if name.endswith("config") else None)
    args = parser.parse_args()
    if args.worker_config:
        return run_worker(args.worker_config)
    if args.driver_config:
        return run_driver(args.driver_config)
    return public_run(args)


if __name__ == "__main__":
    raise SystemExit(main())
