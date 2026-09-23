#!/usr/bin/env python3
"""显式运行已编译的 Rust Codex 适配器验收，按用例隔离认证并保存脱敏证据。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time


TEST_CASES = {
    "lifecycle": "ai::cli_agent_runtime::codex::live_tests::real_codex_managed_lifecycle",
    "running-tool-cancel": "ai::cli_agent_runtime::codex::live_tests::real_codex_running_tool_cancel",
    "local-tools-restore": "ai::cli_agent_runtime::codex::live_tests::real_codex_local_tool_restore",
    "image-input": "ai::cli_agent_runtime::codex::live_tests::real_codex_image_input",
    "missing-session": "ai::cli_agent_runtime::codex::tests::live_codex_missing_session_is_not_replaced",
    "idle-crash": "ai::cli_agent_runtime::codex::tests::idle_crash::live_codex_idle_crash_after_ready_disconnects_once",
}


UNAUTHENTICATED_CASES = {"missing-session", "idle-crash"}
VERSION_PROBE_TIMEOUT_SECONDS = 30


def verified_idle_macos_boundary(event):
    receipt = event.get("exit_receipt", {})
    if not isinstance(receipt, dict):
        return False
    if receipt.get("containment") == "macos_resource_coalition":
        return (receipt.get("cleanup_confirmed") is True
                and event.get("idle_process_cleanup_confirmed") is True
                and event.get("unsafe_recovery_prevented") is False
                and event.get("macos_coalition_ownership_verified") is True
                and event.get("macos_cleanup_proof_verified") is True
                and event.get("running_tool_tree_cleanup_verified") is False)
    if receipt.get("containment") == "unix_process_group":
        # 旧负向记录只能证明恢复被阻止，不能被新域验收升级为完整清理。
        return (receipt.get("cleanup_confirmed") is False
                and event.get("idle_process_cleanup_confirmed") is False
                and event.get("unsafe_recovery_prevented") is True
                and event.get("running_tool_tree_cleanup_verified") is False)
    return True


def verified_acceptance(test_case, exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    if test_case == "lifecycle":
        return any(event.get("event") == "acceptance_passed" for event in events)
    if test_case == "running-tool-cancel":
        expected_order = ["native_item_started", "interrupt_sent", "interrupt_accepted",
                          "turn_finished", "disconnected"]
        return any(event.get("event") == "running_tool_cancel_finished"
                   and event.get("passed") is True
                   and event.get("native_item_started_before_interrupt") is True
                   and event.get("interrupt_native_ack") is True
                   and event.get("same_generation_receipt") is True
                   and event.get("cleanup_confirmed") is True
                   and event.get("tool_tree_zero_residual") is True
                   and event.get("terminal_before_disconnected") is True
                   and event.get("event_order") == expected_order
                   and event.get("containment") in {"macos_resource_coalition", "linux_subtree", "windows_job"}
                   for event in events)
    if test_case == "missing-session":
        return any(event.get("event") == "missing_session_probe_finished" and event.get("passed") is True for event in events)
    if test_case == "idle-crash":
        return any(event.get("event") == "idle_crash_probe_finished" and event.get("passed") is True
                   and event.get("phase") == "after_session_ready" and event.get("native_root_exit_observed") is True
                   and event.get("credentials_provided") is False and event.get("model_commands_sent") == 0
                   and verified_idle_macos_boundary(event)
                   for event in events)
    if test_case == "image-input":
        return any(event.get("event") == "image_probe_finished" and event.get("passed") is True for event in events)
    return any(event.get("event") == "tool_restore_probe_finished" and event.get("passed") is True for event in events)


def missing_session_environment(root):
    # 从最小白名单构造环境，不读取认证文件，也不继承密钥、代理或注入式配置。
    allowed = {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG", "LC_ALL"}
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    paths = {
        "HOME": root / "home", "USERPROFILE": root / "home",
        "APPDATA": root / "home/AppData/Roaming", "LOCALAPPDATA": root / "home/AppData/Local",
        "XDG_CONFIG_HOME": root / "home/.config", "XDG_DATA_HOME": root / "home/.local/share",
        "XDG_CACHE_HOME": root / "home/.cache", "CODEX_HOME": root / "codex",
        "TMPDIR": root / "tmp", "TMP": root / "tmp", "TEMP": root / "tmp",
    }
    for key, path in paths.items():
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        environment[key] = str(path)
    # file 模式禁止回退到系统钥匙串；这个私有 CODEX_HOME 没有 auth.json。
    (root / "codex/config.toml").write_text('cli_auth_credentials_store = "file"\n', encoding="utf-8", newline="\n")
    (root / ".infinishell-missing-session-probe").write_text(
        "isolated unauthenticated missing-session verification\n", encoding="utf-8", newline="\n")
    environment["INFINISHELL_CODEX_MISSING_ROOT"] = str(root)
    return environment


def idle_crash_environment(root):
    environment = missing_session_environment(root)
    environment.pop("INFINISHELL_CODEX_MISSING_ROOT")
    (root / ".infinishell-missing-session-probe").unlink()
    (root / ".infinishell-idle-crash-probe").write_text(
        "isolated unauthenticated idle-crash verification\n", encoding="utf-8", newline="\n")
    environment["INFINISHELL_CODEX_IDLE_CRASH_ROOT"] = str(root)
    return environment


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(4 * 1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def probe_version(executable, environment):
    # Windows 首次启动新复制的官方二进制可能包含防病毒扫描；仍设置有限上限，
    # 但不能让独立的 10 秒预检先于后续 120 秒原生验收误判失败。
    return subprocess.run([str(executable), "--version"], env=environment,
                          text=True, capture_output=True,
                          timeout=VERSION_PROBE_TIMEOUT_SECONDS, check=True)


def terminate(process, own_process_only=False):
    if own_process_only:
        # 只终止运行器持有的 libtest 子进程；原生 CLI 由独立监督者在控制 EOF 后收尾。
        try:
            process.kill()
        except OSError:
            pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        return
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        if os.name != "nt":
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.kill()
        process.wait()


def process_commands():
    # 只在私有测试目录中匹配本次固定脚本；扫描结果不写入证据。
    if os.name == "nt":
        result = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command",
             "Get-CimInstance Win32_Process | Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"],
            capture_output=True, text=True, timeout=10, check=True)
        rows = json.loads(result.stdout)
        if isinstance(rows, dict):
            rows = [rows]
        return {int(row["ProcessId"]): row["CommandLine"] or "" for row in rows}
    result = subprocess.run(["ps", "-Aww", "-o", "pid=", "-o", "command="],
                            capture_output=True, text=True, timeout=10, check=True)
    commands = {}
    for line in result.stdout.splitlines():
        parts = line.strip().split(maxsplit=1)
        if len(parts) == 2 and parts[0].isdigit():
            commands[int(parts[0])] = parts[1]
    return commands


def owned_fixture_pids(root, commands):
    scripts = [str(root / "project" / name) for name in ("cancel-parent.py", "cancel-leaf.py")]
    return {pid for pid, command in commands.items()
            if pid != os.getpid() and any(script in command for script in scripts)}


def owned_supervisor_pids(root, supervisor, commands):
    marker = str(root / "cli-agent-processes")
    return {pid for pid, command in commands.items()
            if pid != os.getpid() and str(supervisor) in command
            and "cli-agent-supervisor" in command and marker in command}


def macos_native_root_pids(root, codex):
    if sys.platform != "darwin":
        return set()
    processes = root / "cli-agent-processes"
    if not processes.is_dir():
        return set()
    owned = set()
    for claim in processes.glob("*/macos-native.json"):
        if claim.stat().st_size > 4096:
            raise ValueError("原生进程身份记录超出固定大小")
        native = json.loads(claim.read_text())
        manifest = json.loads((claim.parent / "manifest.json").read_text())
        if native.get("generation") != claim.parent.name or Path(manifest["executable"]).resolve() != codex:
            raise ValueError("原生进程身份与本次 CLI 启动不匹配")
        pid = native.get("identity", {}).get("pid")
        if type(pid) is not int or pid <= 0:
            raise ValueError("原生进程身份无效")
        owned.add(pid)
    return owned


def owned_native_root_pids(root, codex, candidates):
    if not candidates:
        return set()
    # 原生记录的 PID、官方可执行路径和私有 cwd 均匹配时，才可强制清理本次根进程。
    commands = process_commands()
    owned = set()
    for pid in candidates:
        if str(codex) not in commands.get(pid, ""):
            continue
        result = subprocess.run(["lsof", "-a", "-p", str(pid), "-d", "cwd", "-Fn"],
                                capture_output=True, text=True, timeout=5, check=False)
        if f"n{root / 'project'}" in result.stdout.splitlines():
            owned.add(pid)
    return owned


def audit_and_cleanup_fixture(root, codex, supervisor):
    # 本函数只给安全兜底和独立残留审计使用；其结果绝不替代产品 exit receipt。
    audit = {"fallback_attempted": False, "zero_residual": False,
             "unverified_native_root": False, "audit_error": False,
             "observed_tool_processes": 0, "observed_native_roots": 0,
             "observed_supervisors": 0, "residual_tool_processes": 0,
             "residual_native_roots": 0, "residual_supervisors": 0}
    try:
        native_candidates = macos_native_root_pids(root, codex)
        deadline = time.monotonic() + 20
        while True:
            commands = process_commands()
            tools = owned_fixture_pids(root, commands)
            roots = owned_native_root_pids(root, codex, native_candidates)
            supervisors = owned_supervisor_pids(root, supervisor, commands)
            audit["observed_tool_processes"] = max(audit["observed_tool_processes"], len(tools))
            audit["observed_native_roots"] = max(audit["observed_native_roots"], len(roots))
            audit["observed_supervisors"] = max(audit["observed_supervisors"], len(supervisors))
            if not tools and not roots and not supervisors:
                audit["zero_residual"] = True
                break
            if time.monotonic() >= deadline:
                audit["fallback_attempted"] = True
                for pid in tools | roots | supervisors:
                    # 每次发送强制信号前重新核对本次随机目录或私有 CODEX_HOME 身份。
                    current = process_commands()
                    confirmed_tools = owned_fixture_pids(root, current)
                    confirmed_roots = owned_native_root_pids(root, codex, native_candidates)
                    confirmed_supervisors = owned_supervisor_pids(root, supervisor, current)
                    if pid not in confirmed_tools | confirmed_roots | confirmed_supervisors:
                        continue
                    if os.name == "nt":
                        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                       timeout=5, check=False)
                    else:
                        try:
                            os.kill(pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                verify_deadline = time.monotonic() + 10
                while True:
                    current = process_commands()
                    remaining = owned_fixture_pids(root, current)
                    remaining |= owned_native_root_pids(root, codex, native_candidates)
                    remaining |= owned_supervisor_pids(root, supervisor, current)
                    if not remaining or time.monotonic() >= verify_deadline:
                        audit["zero_residual"] = not remaining
                        break
                    time.sleep(0.25)
                break
            time.sleep(0.25)
        final_commands = process_commands()
        audit["residual_tool_processes"] = len(owned_fixture_pids(root, final_commands))
        audit["residual_native_roots"] = len(owned_native_root_pids(root, codex, native_candidates))
        audit["residual_supervisors"] = len(owned_supervisor_pids(root, supervisor, final_commands))
        audit["zero_residual"] = (audit["residual_tool_processes"] == 0
                                  and audit["residual_native_roots"] == 0
                                  and audit["residual_supervisors"] == 0)
        alive = native_candidates & final_commands.keys()
        audit["unverified_native_root"] = bool(alive - owned_native_root_pids(root, codex, native_candidates))
    except Exception:
        audit["audit_error"] = True
    if audit["audit_error"] or audit["unverified_native_root"]:
        audit["zero_residual"] = False
    return audit


def sanitize(text, root):
    for directory in (root, root.resolve(), Path.home()):
        text = text.replace(str(directory), "<probe-root>" if directory != Path.home() else "<user-home>")
    return re.sub(r"(?:sk-[A-Za-z0-9_-]{16,}|Bearer [A-Za-z0-9_.-]+)", "<redacted>", text)


def stopped_output(process, metadata):
    try:
        return process.communicate(timeout=10)[0]
    except subprocess.TimeoutExpired:
        # 子孙进程误持有 libtest 的输出管道时也不能无限等待。
        if process.stdout is not None:
            try:
                process.stdout.close()
            except OSError:
                pass
        metadata["test_output_incomplete"] = True
        return ""


def run(args):
    repository = Path(__file__).resolve().parents[2]
    test_name = TEST_CASES[args.test_case]
    metadata = {
        "test": test_name,
        "test_case": args.test_case,
        "scope": {"running-tool-cancel": "rust_adapter_running_tool_cancel",
                  "image-input": "rust_adapter_image_input", "missing-session": "rust_adapter_missing_session",
                  "idle-crash": "rust_adapter_idle_crash_after_native_ready"}.get(
            args.test_case, "rust_adapter_process_restart"),
        "credentials_provided": args.test_case not in UNAUTHENTICATED_CASES,
        "model_requests_expected": args.test_case not in UNAUTHENTICATED_CASES,
        "app_restart_and_ui_verified": False,
        "test_binary_sha256": digest(args.test_binary),
        "supervisor_binary_sha256": digest(args.supervisor),
        "supervised_process_lifecycle": True,
        "platform": sys.platform,
    }
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=repository,
                            text=True, capture_output=True, check=True)
    metadata["commit"] = commit.stdout.strip()
    status = subprocess.run(["git", "status", "--porcelain"], cwd=repository,
                            text=True, capture_output=True, check=True)
    metadata["worktree_dirty"] = bool(status.stdout.strip())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary_parent = os.environ.get("RUNNER_TEMP") if args.test_case in UNAUTHENTICATED_CASES else None
    with tempfile.TemporaryDirectory(prefix="infinishell-codex-adapter-", dir=temporary_parent) as temporary:
        root = Path(temporary).resolve()
        configuration = root / "codex"
        configuration.mkdir(mode=0o700)
        (root / "project").mkdir()
        (root / ".infinishell-live-probe").write_text("isolated Rust adapter verification\n")
        if args.test_case in UNAUTHENTICATED_CASES:
            environment = (idle_crash_environment(root) if args.test_case == "idle-crash"
                           else missing_session_environment(root))
        else:
            credentials = configuration / "auth.json"
            with credentials.open("xb") as target:
                credentials.chmod(0o600)
                target.write(args.credential_source.read_bytes())
            environment = os.environ.copy()
            for key in list(environment):
                if any(part in key for part in ("TOKEN", "API_KEY", "AUTH", "SECRET")):
                    environment.pop(key)
        # 环境只传给单独的测试进程；Rust 测试和其他并行测试都不修改全局环境。
        environment.update({
            "CODEX_HOME": str(configuration),
            "INFINISHELL_CODEX_LIVE_ROOT": str(root),
            "INFINISHELL_CODEX_LIVE_EXECUTABLE": str(args.codex),
            "INFINISHELL_CODEX_LIVE_PYTHON": str(Path(sys.executable).resolve()),
            "INFINISHELL_CODEX_LIVE_ARTIFACT": str(args.output),
            "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor),
        })
        if args.test_case == "running-tool-cancel":
            environment["INFINISHELL_CODEX_RUNNING_TOOL_CANCEL"] = "1"
        version = probe_version(args.codex, environment)
        metadata["cli_version"] = version.stdout.strip()
        # 清空本次输出，避免筛选器未匹配或版本不符时采用上一次的成功记录。
        args.output.write_text("")
        if args.test_case == "running-tool-cancel" and metadata["cli_version"] != "codex-cli 0.155.1":
            metadata["acceptance_passed"] = False
            metadata["version_mismatch"] = True
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
            return 1
        command = [str(args.test_binary), test_name, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        platform_options = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt" else {"start_new_session": True}
        process = subprocess.Popen(command, cwd=repository, env=environment,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding="utf-8", **platform_options)
        try:
            output, _ = process.communicate(timeout=120 if args.test_case in UNAUTHENTICATED_CASES else 900)
        except subprocess.TimeoutExpired:
            terminate(process, own_process_only=args.test_case in {"idle-crash", "running-tool-cancel"})
            output = stopped_output(process, metadata) if args.test_case == "running-tool-cancel" else process.communicate()[0]
            metadata["timed_out"] = True
        except BaseException:
            terminate(process, own_process_only=args.test_case in {"idle-crash", "running-tool-cancel"})
            if args.test_case != "running-tool-cancel":
                raise
            output = stopped_output(process, metadata)
            metadata["runner_interrupted"] = True
        if args.test_case == "running-tool-cancel":
            metadata["harness_cleanup"] = audit_and_cleanup_fixture(root, args.codex, args.supervisor)
            metadata["libtest_reaped"] = process.poll() is not None
        metadata["test_exit_code"] = process.returncode
        output_path = args.output.with_suffix(".test-output.txt")
        output_path.write_text(sanitize(output, root), encoding="utf-8")
        # 测试筛选器没有匹配时 libtest 仍返回 0，因此还必须核验真实验收终态。
        events = []
        if args.output.exists():
            try:
                events = [json.loads(line) for line in args.output.read_text().splitlines()]
            except (OSError, UnicodeError, json.JSONDecodeError):
                metadata["invalid_evidence"] = True
        metadata["acceptance_passed"] = (not metadata.get("timed_out", False)
                                         and not metadata.get("runner_interrupted", False)
                                         and not metadata.get("invalid_evidence", False)
                                         and (args.test_case != "running-tool-cancel" or (
                                             metadata["libtest_reaped"]
                                             and
                                             metadata["harness_cleanup"]["zero_residual"]
                                             and not metadata["harness_cleanup"]["fallback_attempted"]
                                             and not metadata["harness_cleanup"]["unverified_native_root"]
                                             and not metadata["harness_cleanup"]["audit_error"]))
                                         and verified_acceptance(args.test_case, process.returncode, output, events))
        args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print("真实 Rust Codex 适配器验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
        print(f"证据：{args.output}")
        return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-case", choices=TEST_CASES, default="lifecycle", help="选择生命周期、运行中工具取消、本地工具保存恢复、图片、无凭据缺失会话或原生空闲崩溃验收")
    parser.add_argument("--test-binary", type=Path, required=True, help="cargo test --no-run 生成的 warp libtest 可执行文件")
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True, help="与适配器代码同提交构建、支持隐藏 worker 的 InfiniShell 主程序或 TUI 二进制")
    parser.add_argument("--credential-source", type=Path, help="有模型用例必填；missing-session/idle-crash 禁止提供认证来源")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.test_case in UNAUTHENTICATED_CASES:
        if args.credential_source is not None:
            parser.error(f"{args.test_case} 不接受认证来源")
    elif args.credential_source is None:
        parser.error("有模型用例必须提供已有 credential-source")
    for name in ("test_binary", "codex", "credential_source", "supervisor"):
        if getattr(args, name) is None:
            continue
        path = getattr(args, name).resolve(strict=True)
        if not path.is_file():
            parser.error(f"{name} 必须是现有文件")
        setattr(args, name, path)
    args.output = args.output.resolve()
    if args.supervisor == args.test_binary:
        parser.error("监督入口必须是主程序或 TUI 二进制，不能指向 libtest")
    inputs = {path for path in (args.test_binary, args.codex, args.credential_source, args.supervisor) if path is not None}
    artifacts = {args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".test-output.txt")}
    if inputs & artifacts:
        parser.error("证据输出不得覆盖可执行文件或只读认证来源")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
