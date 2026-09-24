#!/usr/bin/env python3
"""固定三款 CLI 的隔离升级：Linux/Windows 均复用产品 inspect/execute/journal 事务。"""
import argparse
import copy
from contextlib import contextmanager, nullcontext
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace

import prepare_claude_cli as claude
import prepare_codex_cli as codex
import prepare_grok_cli as grok
import verify_cli_autoupdate as transaction

REPO = Path(__file__).resolve().parents[2]
OLD = {"codex": "0.155.1", "claude": "2.1.278", "grok": "1.0.40"}
TARGET = {"codex": "0.156.1", "claude": "2.1.280", "grok": "1.0.41"}
PLUGIN = {"codex": "0.147.0", "claude": "2.1.273", "grok": "1.0.41"}
PREPARE_TEST = "terminal::cli_agent_updates::sources::live_tests::prepare_native_update_plugins_without_model"
WINDOWS_PRODUCT_TEST = "terminal::cli_agent_updates::sources::windows_live_tests::real_native_update_without_model"
WINDOWS_TEST = "ai::cli_agent_runtime::managed_process::atomic_windows::tests::debug_session_runs_fixed_real_cli_to_native_exit"
CASES = ("updated", "source_changed_rejected", "command_failed_rolled_back", "interrupted_recovered")


def require(value, message):
    if not value:
        raise ValueError(message)


def write(path, value):
    transaction.exclusive_bytes(path, transaction.encoded(value))


def binding(path):
    return {"path": str(path), "sha256": transaction.digest(path)}


def record_build_inputs(args):
    # 复制任何原生夹具前保存构建文件形状，避免把超大调试产物误报为非普通文件。
    binaries = []
    for role, path in (("worker", args.test_binary), ("supervisor", args.supervisor)):
        row = {"role": role, "path": str(path.absolute()), "is_file": path.is_file(),
               "bytes": None, "stat_error_type": None}
        try:
            row["bytes"] = path.stat().st_size
        except OSError as error:
            row["stat_error_type"] = type(error).__name__
        binaries.append(row)
    write(args.output.with_name("build-inputs.safe.json"), {
        "schema_version": 1, "scope": "固定升级夹具复制前的构建文件元数据",
        "binary_limit_bytes": transaction.MAX_BINARY_BYTES, "binaries": binaries,
        "credentials_provided": False, "model_inputs_sent": 0})


def prepared_input(agent, version, cache):
    script, flag = {"codex": ("prepare_codex_cli.py", "--version"),
                    "claude": ("prepare_claude_cli.py", "--claude-version"),
                    "grok": ("prepare_grok_cli.py", "--version")}[agent]
    process = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name(script)), flag, version,
                              "--download-dir", str(cache / f"{agent}-{version}")],
                             capture_output=True, text=True, timeout=600, check=False)
    require(process.returncode == 0, "fixed_input_preparation_failed")
    path = Path(process.stdout.strip()).resolve(strict=True)
    require(path.is_relative_to(cache.resolve()), "fixed_input_outside_cache")
    return path


def verified_input(agent, version, executable):
    target = "linux-x64" if sys.platform == "linux" else "win32-x64"
    if agent == "codex":
        package_target = "linux-x64" if sys.platform == "linux" else "windows-x64"
        package = codex.packages_for_version(version)[package_target]
        root = executable.parent.parent
        require(codex.verify_runtime_tree(root, package, version).resolve() == executable,
                "fixed_codex_entry_mismatch")
    elif agent == "claude":
        claude.verify_binary(executable, target, version)
    else:
        grok.verify_binary(executable, target, version)
    return binding(executable)


def copy_input(agent, source, destination):
    if agent == "codex":
        destination.parent.parent.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copytree(source.parent.parent, destination.parent.parent, dirs_exist_ok=False)
    else:
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    return destination


def windows_junction(link, target):
    # 只供私有夹具使用；拒绝 cmd 展开字符，避免路径变成命令。
    require(sys.platform == "win32", "junction_requires_windows")
    require(all(path.is_absolute() and not any(character in str(path) for character in '%!&|^<>\r\n"')
                for path in (link, target)), "junction_path_not_safe")
    require(not link.exists() and not link.is_symlink(), "junction_entry_exists")
    link.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    command = Path(os.environ["SystemRoot"]) / "System32/cmd.exe"
    result = subprocess.run([str(command), "/D", "/V:OFF", "/C", "mklink", "/J", str(link), str(target)],
                            capture_output=True, timeout=30, check=False)
    require(result.returncode == 0 and link.resolve(strict=True) == target.resolve(strict=True),
            "junction_target_mismatch")


@contextmanager
def windows_codex_user_path(entry):
    # 官方安装器会持久化 User Path。临时预置私有入口使安装器无需改写；
    # 只在值仍等于本次写入时恢复原始值/类型，绝不覆盖并发修改，也不输出其内容。
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0,
                        winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE) as key:
        try:
            original = winreg.QueryValueEx(key, "Path")
        except FileNotFoundError:
            original = None
        require(original is None or isinstance(original[0], str) and original[1] in (winreg.REG_SZ, winreg.REG_EXPAND_SZ),
                "windows_user_path_type_unsupported")
        kind = original[1] if original is not None else winreg.REG_EXPAND_SZ
        value = str(entry.parent) + (";" + original[0] if original is not None and original[0] else "")
        temporary = (value, kind)
        winreg.SetValueEx(key, "Path", 0, kind, value)
        try:
            yield
        finally:
            try:
                current = winreg.QueryValueEx(key, "Path")
            except FileNotFoundError:
                current = None
            require(current == temporary, "windows_user_path_changed_concurrently")
            if original is None:
                winreg.DeleteValue(key, "Path")
            else:
                winreg.SetValueEx(key, "Path", 0, original[1], original[0])


def fixture(agent, old, target, parent):
    root = Path(tempfile.mkdtemp(prefix=f"infinishell-cli-autoupdate-{agent}-", dir=parent)).resolve()
    windows = sys.platform == "win32"
    suffix = ".exe" if windows else ""
    for part in ("home/.local/bin", "home/.codex", "home/.claude", "home/.grok/bin", "references", "project", "tmp"):
        (root / part).mkdir(mode=0o700, parents=True, exist_ok=True)
    if agent == "codex":
        platform = "x86_64-pc-windows-msvc" if windows else "x86_64-unknown-linux-musl"
        package = root / "home/.codex/packages/standalone/releases" / f"{OLD[agent]}-{platform}"
        old_copy = copy_input(agent, old, package / "bin" / f"codex{suffix}")
        target_copy = copy_input(agent, target, root / "references/target/bin" / f"codex{suffix}")
        current = root / "home/.codex/packages/standalone/current"
        if windows:
            # 对齐固定官方安装器：current 与可见 bin 均是目录联接。
            windows_junction(current, package)
            visible = root / "home/AppData/Local/Programs/OpenAI/Codex/bin"
            windows_junction(visible, current / "bin")
            entry = visible / "codex.exe"
        else:
            current.symlink_to(package, target_is_directory=True)
            entry = root / "home/.local/bin/codex"
            entry.symlink_to(current / "bin/codex")
        (root / "home/.codex/config.toml").write_text("# isolated update fixture\n")
    elif agent == "claude":
        old_copy = copy_input(agent, old, root / "home/.local/share/claude/versions" / OLD[agent])
        target_copy = copy_input(agent, target, root / "references" / f"claude{suffix}")
        entry = root / "home/.local/bin" / f"claude{suffix}"
        (root / "home/.claude/settings.json").write_text('{"autoUpdatesChannel":"latest"}\n')
    else:
        platform = "windows-x86_64.exe" if windows else "linux-x86_64"
        old_copy = copy_input(agent, old, root / "home/.grok/downloads" / f"grok-{OLD[agent]}-{platform}")
        target_copy = copy_input(agent, target, root / "references" / f"grok-{TARGET[agent]}-{platform}")
        entry = root / "home/.grok/bin" / f"grok{suffix}"
        (root / "home/.grok/config.toml").write_text('[cli]\ninstaller="internal"\nchannel="stable"\nauto_update=false\n')
    if agent != "codex":
        if windows:
            shutil.copy2(old_copy, entry)
        else:
            entry.symlink_to(old_copy)
    for config in (root / "home/.codex/config.toml", root / "home/.claude/settings.json", root / "home/.grok/config.toml"):
        if config.exists():
            config.chmod(0o600)
    require(transaction.digest(old_copy) == transaction.digest(old), "old_copy_digest")
    require(transaction.digest(target_copy) == transaction.digest(target), "target_copy_digest")
    return root, old_copy, target_copy, entry


def build_records(root, worker, supervisor):
    source = root / "source-manifest.safe.json"
    write(source, {"files": [{"path": path, "bytes": (REPO / path).stat().st_size,
                              "sha256": transaction.digest(REPO / path)} for path in transaction.REQUIRED_SOURCE_FILES]})
    gates = root / "gates.safe.json"
    bundle = root / "bundle.safe.json"
    write(gates, {"source_manifest_sha256": transaction.digest(source), "test_binary": binding(worker),
                  "scope": "同提交 CI 已成功的 check 与 focused 步骤；执行文件另行摘要绑定"})
    write(bundle, {"source_manifest_sha256": transaction.digest(source), "worker": binding(supervisor)})
    return {"source_manifest": binding(source), "gates_report": binding(gates), "bundle_report": binding(bundle)}


def prepare_plugins(root, value, plugin):
    entry = Path(value["entry"])
    original_link = entry.readlink()
    original_manifest = (root / "manifest.private.json").read_bytes()
    prepared = copy.deepcopy(value)
    reference = root / "references/plugin/bin" / entry.name if value["agent"] == "codex" else root / "references/plugin" / entry.name
    copied = copy_input(value["agent"], plugin, reference)
    prepared["old_binary"] = binding(copied)
    prepared["old_version"] = PLUGIN[value["agent"]]
    environment = transaction.isolated_environment(root, prepared)
    process = None
    try:
        entry.unlink()
        entry.symlink_to(copied)
        (root / "manifest.private.json").write_bytes(transaction.encoded(prepared))
        with (root / "plugin-preparation.private.txt").open("xb") as log:
            process = subprocess.Popen([value["worker"]["path"], "--exact", PREPARE_TEST, "--ignored", "--nocapture", "--test-threads=1"],
                                       cwd=root / "project", env=environment, stdin=subprocess.DEVNULL,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            code = process.wait(timeout=300)
        receipt = root / "plugin-preparation.safe.json"
        return code == 0 and receipt.is_file() and json.loads(receipt.read_bytes()).get("passed") is True
    finally:
        try:
            if process is not None:
                require(transaction.stop_group(process), "plugin_process_group_not_stopped")
        finally:
            entry.unlink()
            entry.symlink_to(original_link)
            (root / "manifest.private.json").write_bytes(original_manifest)


def linux_case(args, agent, old, target, plugin, expected):
    root, old, target, entry = fixture(agent, old, target, args.fixture_parent)
    for dependency in ("node", "jq"):
        resolved = shutil.which(dependency)
        require(resolved, "plugin_dependency_missing")
        (root / "home/.local/bin" / dependency).symlink_to(Path(resolved).resolve())
    value = {"schema": 1, "scope": transaction.SCOPE, "case_id": f"{agent}-{expected.replace('_', '-')}-{root.name[-8:].replace('_', '-')}",
             "root": str(root), "agent": agent, "channel": "follow_installation", "expected": expected,
             "entry": str(entry), "old_version": OLD[agent], "target_version": TARGET[agent],
             "old_binary": binding(old), "target_binary": binding(target), "worker": binding(args.test_binary),
             "supervisor": binding(args.supervisor), **build_records(root, args.test_binary, args.supervisor),
             "timeout_seconds": 480, "fixed_release_input": True, "test_only_target_candidate": False}
    write(args.output.parent / f"{agent}-{expected}-input.safe.json", {
        "scope": "Linux 固定官方输入的产品事务", "agent": agent, "expected": expected,
        "old_version": OLD[agent], "target_version": TARGET[agent], "fixture_root": str(root),
        "old_binary": value["old_binary"], "target_binary": value["target_binary"],
        "test_binary": value["worker"], "supervisor": value["supervisor"],
        "source_manifest_sha256": value["source_manifest"]["sha256"],
        "fixed_release_input": True, "production_gate_claimed": True,
        "model_inputs_sent": 0, "credentials_provided": False})
    transaction.verify_fixture(value, require_marker=False)
    transaction.isolated_environment(root, value)
    transaction.exclusive_bytes(root / ".infinishell-cli-autoupdate", transaction.MARKER)
    write(root / "manifest.private.json", value)
    require(prepare_plugins(root, value, plugin), "plugin_preparation_failed")
    result = transaction.run(SimpleNamespace(manifest=root / "manifest.private.json", case_id=value["case_id"], allow_native_update=True))
    snapshots = root / "home/.codex/packages/standalone/releases/cli-agent-executable-snapshots"
    snapshots_reclaimed = not snapshots.exists() or snapshots.is_dir() and not any(snapshots.iterdir())
    event = root / "events.safe.json"
    event_value = json.loads(transaction.private_bytes(event)) if event.exists() else None
    safe_event = transaction.valid_event(event_value)
    return {"agent": agent, "expected": expected, "passed": result["passed"] and snapshots_reclaimed,
            "failure_code": event_value["failure_code"] if safe_event else None,
            "product_stage": event_value["stage"] if safe_event else None,
            "product_error": event_value["error"] if safe_event else None,
            "layout_snapshots_reclaimed": snapshots_reclaimed, "receipt": binding(root / "receipt.safe.json"),
            "event": binding(event) if safe_event else None,
            "plugin_preparation": binding(root / "plugin-preparation.safe.json") }


def windows_environment(root):
    environment = grok.isolated_environment(root)
    # 所有显式 updater 安装根均位于本次隔离 HOME；只保留系统程序目录。
    system = Path(os.environ["SystemRoot"])
    environment["PATH"] = os.pathsep.join(str(path) for path in (system / "System32", system / "System32/WindowsPowerShell/v1.0", system))
    environment.update(CODEX_HOME=str(root / "home/.codex"), CLAUDE_CONFIG_DIR=str(root / "home/.claude"),
                       GROK_HOME=str(root / "home/.grok"), CODEX_INSTALL_DIR=str(root / "home/AppData/Local/Programs/OpenAI/Codex/bin"),
                       CODEX_RELEASE=TARGET["codex"], CODEX_NON_INTERACTIVE="1", DISABLE_AUTOUPDATER="1",
                       CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1")
    return environment


def windows_debug_case(args, agent, old, target):
    root, old, target, entry = fixture(agent, old, target, args.fixture_parent)
    environment = windows_environment(root)
    arguments = {"codex": ["update"], "claude": ["--settings", '{"autoUpdatesChannel":"latest"}', "install", TARGET["claude"]],
                 "grok": ["update", "--version", TARGET["grok"]]}[agent]
    configuration = {path: transaction.digest(path) for path in root.joinpath("home").rglob("*") if path.is_file() and path.suffix in (".json", ".toml")}
    old_sha, target_sha = transaction.digest(old), transaction.digest(target)
    environment.update(INFINISHELL_WINDOWS_REAL_CLI_FIXTURE=str(old), INFINISHELL_WINDOWS_REAL_CLI_ARGS=json.dumps(arguments),
                       INFINISHELL_WINDOWS_REAL_CLI_ENV=json.dumps(environment))
    log_path = root / "atomic-update.private.txt"
    timed_out = False
    with windows_codex_user_path(entry) if agent == "codex" else nullcontext():
        with log_path.open("xb") as log:
            process = subprocess.Popen([str(args.test_binary), WINDOWS_TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                       cwd=root / "project", env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            try:
                process.wait(timeout=390)
            except subprocess.TimeoutExpired:
                timed_out = True
                subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"), "/PID", str(process.pid), "/T", "/F"],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)
                process.wait(timeout=15)
    log = log_path.read_text(encoding="utf-8", errors="replace")
    errors = sorted(set(re.findall(r"managed_process\.atomic_windows_[a-z_]+", log)))
    rejected_images = []
    debug_failures = []
    for line in log.splitlines():
        if line.startswith("atomic_windows_debug_failure=") and len(debug_failures) < 4:
            try:
                failure = json.loads(line.split("=", 1)[1])
            except json.JSONDecodeError:
                continue
            if (isinstance(failure, dict) and set(failure) == {"stage", "os_error", "hresult", "failure_code"}
                    and failure["stage"] in ("begin_session", "wait_for_exit")
                    and all(failure[key] is None or type(failure[key]) is int and -2**31 <= failure[key] < 2**31
                            for key in ("os_error", "hresult"))
                    and (failure["failure_code"] is None or isinstance(failure["failure_code"], str)
                         and re.fullmatch(r"managed_process\.atomic_windows_[a-z_]{1,98}", failure["failure_code"]))):
                debug_failures.append(failure)
            continue
        if not line.startswith("atomic_windows_rejected_image=") or len(rejected_images) >= 8:
            continue
        try:
            image = json.loads(line.split("=", 1)[1])
        except json.JSONDecodeError:
            continue
        if (isinstance(image, dict) and set(image) == {"basename", "directory_category"}
                and (image["basename"] is None or isinstance(image["basename"], str)
                     and re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", image["basename"]))
                and image["directory_category"] in ("windows_microsoft_net", "windows_winsxs",
                    "windows_system32_subdirectory", "windows_other", "outside_windows_or_unresolved")):
            rejected_images.append(image)
    native_exit = process.returncode == 0 and "1 passed; 0 failed" in log and "严格 Job 已清空" in log
    strict_job_cleanup = not timed_out and "atomic_windows_strict_job_cleanup_confirmed=true" in log.splitlines()
    native_status = re.search(r"真实更新参数的原生退出状态：exit code: (0x[0-9a-fA-F]+|[0-9]+)(?=\s|\(|$)", log)
    native_exit_code = int(native_status[1], 16 if native_status[1].startswith("0x") else 10) if native_status else None
    updater_succeeded = native_exit_code == 0
    config_unchanged = all(path.exists() and transaction.digest(path) == checksum for path, checksum in configuration.items())
    entry_matches = entry.is_file() and transaction.digest(entry) == target_sha
    version_matches = False
    if entry_matches:
        version = subprocess.run([str(entry), "--version"], cwd=root / "project", env=windows_environment(root),
                                 stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20, check=False)
        expected_version = {"codex": f"codex-cli {TARGET[agent]}", "claude": f"{TARGET[agent]} (Claude Code)",
                            "grok": grok.VERSION_OUTPUTS[TARGET["grok"]]}[agent]
        version_matches = version.returncode == 0 and version.stdout.strip() == expected_version
    evidence = {"scope": "Windows 固定官方 CLI 严格 Job 原子执行诊断；不是产品事务或正式版本支持验收",
                "agent": agent, "old_version": OLD[agent], "target_version": TARGET[agent], "candidate": True,
                "native_update_attempted": True, "model_inputs_sent": 0, "credentials_provided": False, "user_path_restored": True,
                "user_path_temporary_entry_used": agent == "codex",
                "test_binary": binding(args.test_binary), "supervisor": binding(args.supervisor),
                "source_files": [{"path": path, "sha256": transaction.digest(REPO / path)} for path in (
                    "app/src/ai/cli_agent_runtime/managed_process_atomic_windows.rs",
                    "app/src/ai/cli_agent_runtime/managed_process_atomic_windows_tests.rs")],
                "old_sha256": old_sha, "target_sha256": target_sha,
                "native_exit_and_strict_job_confirmed": native_exit, "native_exit_code": native_exit_code,
                "strict_job_cleanup_confirmed": strict_job_cleanup, "debug_failures": debug_failures,
                "updater_exit_success": updater_succeeded, "timed_out": timed_out,
                "old_binary_unchanged": transaction.digest(old) == old_sha, "target_reference_unchanged": transaction.digest(target) == target_sha,
                "configuration_unchanged": config_unchanged, "entry_matches_target": entry_matches,
                "target_native_version_matches": version_matches, "credentials_absent": transaction.auth_absent(root),
                "stable_errors": errors, "rejected_images": rejected_images,
                "private_log_sha256": transaction.digest(log_path)}
    evidence["passed"] = all(evidence[key] for key in ("native_exit_and_strict_job_confirmed", "strict_job_cleanup_confirmed", "updater_exit_success", "old_binary_unchanged", "target_reference_unchanged",
                                                      "configuration_unchanged", "entry_matches_target", "target_native_version_matches", "credentials_absent")) and not timed_out
    write(root / "receipt.safe.json", evidence)
    return {"agent": agent, "expected": "updated", "passed": evidence["passed"], "receipt": binding(root / "receipt.safe.json")}


WINDOWS_CONFIGS = ("home/.codex/config.toml", "home/.codex/packages/standalone/auto-update-version",
    "home/.grok/config.toml", "home/.claude/settings.json", "home/.claude/.claude.json", "home/.claude.json",
    "home/.profile", "home/.bashrc", "home/.bash_profile", "home/.zshrc", "home/.zprofile", "home/.zshenv", "home/.config/fish/config.fish")
WINDOWS_SOURCES = ("app/src/terminal/cli_agent_updates.rs", "app/src/terminal/cli_agent_updates/sources.rs",
    "app/src/ai/cli_agent_runtime/managed_process.rs", "app/src/ai/cli_agent_runtime/managed_process_atomic_windows.rs",
    "app/src/terminal/cli_agent_updates/sources_windows_live_tests.rs", "script/cli-agent-parity/run_fixed_cli_autoupdate.py")


def windows_case(args, agent, old, target):
    root, old, target, entry = fixture(agent, old, target, args.fixture_parent)
    environment = windows_environment(root)
    value = {"schema": 1, "scope": "cli_autoupdate_windows_native_product", "root": str(root), "agent": agent,
             "entry": str(entry), "old_version": OLD[agent], "target_version": TARGET[agent],
             "old_binary": binding(old), "target_binary": binding(target), "worker": binding(args.test_binary),
             "supervisor": binding(args.supervisor),
             "sources": {path: transaction.digest(REPO / path) for path in WINDOWS_SOURCES},
             "configs": {path: transaction.digest(root / path) if (root / path).is_file() else None for path in WINDOWS_CONFIGS}}
    manifest = root / "manifest.windows.private.json"
    write(manifest, value)
    transaction.exclusive_bytes(root / ".infinishell-cli-autoupdate", transaction.MARKER)
    environment.update(INFINISHELL_CLI_AUTOUPDATE_ALLOW="native-update-no-model",
                       INFINISHELL_CLI_AUTOUPDATE_MANIFEST=str(manifest),
                       INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor))
    log_path = root / "product-update.private.txt"
    timed_out = False
    with windows_codex_user_path(entry) if agent == "codex" else nullcontext():
        with log_path.open("xb") as log:
            process = subprocess.Popen([str(args.test_binary), WINDOWS_PRODUCT_TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                       cwd=root / "project", env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            try:
                process.wait(timeout=480)
            except subprocess.TimeoutExpired:
                timed_out = True
                subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"), "/PID", str(process.pid), "/T", "/F"],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)
                process.wait(timeout=15)
    product_path = root / "receipt.windows.safe.json"
    product = json.loads(product_path.read_bytes()) if product_path.is_file() else {}
    required = ("native_exit_confirmed", "strict_job_cleanup_confirmed", "entry_matches_target", "target_version_matches",
                "config_bytes_unchanged", "journal_absent", "old_binary_unchanged", "target_reference_unchanged", "credentials_absent")
    bindings_match = all(product.get(name) == expected for name, expected in {
        "scope": value["scope"], "agent": agent, "old_version": OLD[agent], "target_version": TARGET[agent],
        "manifest_sha256": transaction.digest(manifest), "worker_sha256": value["worker"]["sha256"],
        "supervisor_sha256": value["supervisor"]["sha256"], "old_sha256": value["old_binary"]["sha256"],
        "target_sha256": value["target_binary"]["sha256"], "product_execute_calls": 1, "product_inspect_calls": 2,
        "config_files_checked": len(WINDOWS_CONFIGS), "fixed_release_input": True,
        "capability_or_source_gate_bypassed": False, "model_inputs_sent": 0, "credentials_provided": False}.items())
    configurations_match = all((transaction.digest(root / path) if (root / path).is_file() else None) == checksum
                               for path, checksum in value["configs"].items())
    target_matches = entry.is_file() and transaction.digest(entry) == value["target_binary"]["sha256"]
    evidence = {"scope": "Windows 固定官方 CLI 产品 inspect/execute/journal 事务；元数据输入和状态目录隔离，不绕过来源或执行门禁",
                "agent": agent, "old_version": OLD[agent], "target_version": TARGET[agent], "candidate": False,
                "live_latest_claimed": False, "model_inputs_sent": 0, "credentials_provided": False,
                "test_binary": binding(args.test_binary), "supervisor": binding(args.supervisor),
                "manifest_sha256": transaction.digest(manifest), "source_files": value["sources"],
                "product_receipt": binding(product_path) if product_path.is_file() else None,
                "product_receipt_binding_verified": bindings_match, "configuration_unchanged": configurations_match,
                "entry_matches_target": target_matches, "user_path_restored": True,
                "user_path_temporary_entry_used": agent == "codex", "timed_out": timed_out,
                "stable_errors": sorted(set(re.findall(r"managed_process\.atomic_windows_[a-z_]+", log_path.read_text(encoding="utf-8", errors="replace")))),
                "private_log_sha256": transaction.digest(log_path),
                "passed": process.returncode == 0 and not timed_out and product.get("passed") is True
                    and bindings_match and all(product.get(key) is True for key in required) and configurations_match
                    and target_matches and transaction.auth_absent(root)}
    write(root / "receipt.safe.json", evidence)
    return {"agent": agent, "expected": "updated", "passed": evidence["passed"], "receipt": binding(root / "receipt.safe.json"),
            "product_receipt": binding(product_path) if product_path.is_file() else None, "timed_out": timed_out}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fixture-parent", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--cases", nargs="+", choices=CASES, default=list(CASES))
    parser.add_argument("--agents", nargs="+", choices=TARGET, default=list(TARGET))
    args = parser.parse_args()
    require(sys.platform in ("linux", "win32"), "platform_requires_native_linux_or_windows")
    require(not args.output.exists(), "existing_receipt_preserved")
    args.output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.output = args.output.absolute()
    record_build_inputs(args)
    args.test_binary = args.test_binary.resolve(strict=True)
    args.supervisor = args.supervisor.resolve(strict=True)
    args.fixture_parent = args.fixture_parent.resolve(strict=True)
    args.cache.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.cache = args.cache.resolve()
    results = []
    for agent in args.agents:
        preparing_version = OLD[agent]
        try:
            inputs = {}
            versions = [("old", OLD[agent]), ("target", TARGET[agent])]
            if sys.platform == "linux":
                versions.append(("plugin", PLUGIN[agent]))
            for role, version in versions:
                preparing_version = version
                print(json.dumps({"stage": "prepare_started", "agent": agent, "version": version}),
                      file=sys.stderr, flush=True)
                inputs[role] = prepared_input(agent, version, args.cache)
                verified_input(agent, version, inputs[role])
                print(json.dumps({"stage": "prepare_finished", "agent": agent, "version": version,
                                  "passed": True, "failure_code": None}), file=sys.stderr, flush=True)
            old, target, plugin = inputs["old"], inputs["target"], inputs.get("plugin")
            for expected in args.cases if sys.platform == "linux" else ("updated",):
                print(json.dumps({"stage": "case_started", "agent": agent, "old_version": OLD[agent],
                                  "target_version": TARGET[agent], "expected": expected}), file=sys.stderr, flush=True)
                try:
                    result = linux_case(args, agent, old, target, plugin, expected) if sys.platform == "linux" else windows_case(args, agent, old, target)
                    # 只汇出允许上传的结构化收据；原生日志和配置始终留在私有夹具内。
                    for name in ("receipt", "event", "plugin_preparation", "product_receipt"):
                        if result.get(name):
                            source = Path(result[name]["path"])
                            exported = args.output.parent / f"{agent}-{expected}-{name}.safe.json"
                            transaction.exclusive_bytes(exported, source.read_bytes())
                            result[name] = binding(exported)
                    results.append(result)
                    if sys.platform == "win32" and result["passed"] is False and result.get("timed_out") is False:
                        # 产品失败始终保留；仅用另一份隔离副本诊断原生 loader/退出合同。
                        print(json.dumps({"stage": "diagnostic_started", "agent": agent,
                                          "target_version": TARGET[agent]}), file=sys.stderr, flush=True)
                        try:
                            diagnostic = windows_debug_case(args, agent, old, target)
                            source = Path(diagnostic["receipt"]["path"])
                            exported = args.output.parent / f"{agent}-{expected}-debug_receipt.safe.json"
                            transaction.exclusive_bytes(exported, source.read_bytes())
                            diagnostic["receipt"] = binding(exported)
                            result["diagnostic"] = diagnostic
                        except (OSError, ValueError, subprocess.SubprocessError) as failure:
                            result["diagnostic"] = {"passed": False, "failure_type": type(failure).__name__,
                                "failure_code": str(failure) if re.fullmatch(r"[a-z0-9_]{1,80}", str(failure)) else None}
                        print(json.dumps({"stage": "diagnostic_finished", "agent": agent,
                                          "target_version": TARGET[agent], "passed": result["diagnostic"]["passed"],
                                          "failure_code": result["diagnostic"].get("failure_code")}),
                              file=sys.stderr, flush=True)
                except (OSError, ValueError, subprocess.SubprocessError) as failure:
                    results.append({"agent": agent, "expected": expected, "passed": False, "failure_type": type(failure).__name__,
                                    "failure_code": str(failure) if re.fullmatch(r"[a-z0-9_]{1,80}", str(failure)) else None})
                print(json.dumps({"stage": "case_finished", "agent": agent, "old_version": OLD[agent],
                                  "target_version": TARGET[agent], "expected": expected,
                                  "passed": results[-1]["passed"], "failure_code": results[-1].get("failure_code"),
                                  "product_stage": results[-1].get("product_stage"),
                                  "product_error": results[-1].get("product_error")}),
                      file=sys.stderr, flush=True)
        except (OSError, ValueError, subprocess.SubprocessError) as failure:
            results.append({"agent": agent, "expected": "prepare", "passed": False, "failure_type": type(failure).__name__,
                                    "failure_code": str(failure) if re.fullmatch(r"[a-z0-9_]{1,80}", str(failure)) else None})
            print(json.dumps({"stage": "prepare_finished", "agent": agent, "version": preparing_version,
                              "passed": False, "failure_code": results[-1]["failure_code"]}), file=sys.stderr, flush=True)
    value = {"scope": "固定真实原子更新产品事务", "platform": sys.platform,
             "selected_agents": args.agents, "selected_cases": args.cases if sys.platform == "linux" else ["updated"],
             "target_versions": {agent: TARGET[agent] for agent in args.agents},
             "live_latest_claimed": False, "model_inputs_sent": 0, "credentials_provided": False,
             "production_path_exercised": any(row.get("expected") in CASES and row.get("passed") is True for row in results),
             "cases": results, "passed": bool(results) and all(row["passed"] for row in results)}
    write(args.output, value)
    print(json.dumps({"passed": value["passed"], "cases": len(results), "receipt_sha256": transaction.digest(args.output)}))
    return 0 if value["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
