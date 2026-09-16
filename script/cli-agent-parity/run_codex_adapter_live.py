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


TEST_CASES = {
    "lifecycle": "ai::cli_agent_runtime::codex::live_tests::real_codex_managed_lifecycle",
    "local-tools-restore": "ai::cli_agent_runtime::codex::live_tests::real_codex_local_tool_restore",
    "image-input": "ai::cli_agent_runtime::codex::live_tests::real_codex_image_input",
    "missing-session": "ai::cli_agent_runtime::codex::tests::live_codex_missing_session_is_not_replaced",
    "idle-crash": "ai::cli_agent_runtime::codex::tests::idle_crash::live_codex_idle_crash_after_ready_disconnects_once",
}


UNAUTHENTICATED_CASES = {"missing-session", "idle-crash"}


def verified_acceptance(test_case, exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    if test_case == "lifecycle":
        return any(event.get("event") == "acceptance_passed" for event in events)
    if test_case == "missing-session":
        return any(event.get("event") == "missing_session_probe_finished" and event.get("passed") is True for event in events)
    if test_case == "idle-crash":
        return any(event.get("event") == "idle_crash_probe_finished" and event.get("passed") is True
                   and event.get("phase") == "after_session_ready" and event.get("native_root_exit_observed") is True
                   and event.get("credentials_provided") is False and event.get("model_commands_sent") == 0
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


def terminate(process, own_process_only=False):
    if own_process_only:
        # 只终止运行器持有的 libtest 子进程；原生 CLI 由独立监督者在控制 EOF 后收尾。
        process.kill()
        process.wait(timeout=10)
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


def sanitize(text, root):
    for directory in (root, root.resolve(), Path.home()):
        text = text.replace(str(directory), "<probe-root>" if directory != Path.home() else "<user-home>")
    return re.sub(r"(?:sk-[A-Za-z0-9_-]{16,}|Bearer [A-Za-z0-9_.-]+)", "<redacted>", text)


def run(args):
    repository = Path(__file__).resolve().parents[2]
    test_name = TEST_CASES[args.test_case]
    metadata = {
        "test": test_name,
        "test_case": args.test_case,
        "scope": {"image-input": "rust_adapter_image_input", "missing-session": "rust_adapter_missing_session",
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
        version = subprocess.run([str(args.codex), "--version"], env=environment,
                                 text=True, capture_output=True, timeout=10, check=True)
        metadata["cli_version"] = version.stdout.strip()
        # 清空本次输出，避免筛选器未匹配时错误采用上一次的成功记录。
        args.output.write_text("")
        command = [str(args.test_binary), test_name, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        platform_options = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt" else {"start_new_session": True}
        process = subprocess.Popen(command, cwd=repository, env=environment,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding="utf-8", **platform_options)
        try:
            output, _ = process.communicate(timeout=120 if args.test_case in UNAUTHENTICATED_CASES else 900)
        except subprocess.TimeoutExpired:
            terminate(process, own_process_only=args.test_case == "idle-crash")
            output, _ = process.communicate()
            metadata["timed_out"] = True
        except BaseException:
            terminate(process, own_process_only=args.test_case == "idle-crash")
            raise
        metadata["test_exit_code"] = process.returncode
        output_path = args.output.with_suffix(".test-output.txt")
        output_path.write_text(sanitize(output, root), encoding="utf-8")
        # 测试筛选器没有匹配时 libtest 仍返回 0，因此还必须核验真实验收终态。
        events = []
        if args.output.exists():
            events = [json.loads(line) for line in args.output.read_text().splitlines()]
        metadata["acceptance_passed"] = (not metadata.get("timed_out", False)
                                         and verified_acceptance(args.test_case, process.returncode, output, events))
        args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print("真实 Rust Codex 适配器验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
        print(f"证据：{args.output}")
        return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-case", choices=TEST_CASES, default="lifecycle", help="选择生命周期、本地工具保存恢复、图片、无凭据缺失会话或原生空闲崩溃验收")
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
