#!/usr/bin/env python3
"""在私有 HOME 中运行生产 Rust 插件迁移验收；不提供凭据、不注册 hook、不调用模型。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile


TEST_NAME = ("terminal::cli_agent_sessions::plugin_manager::codex_source::tests::"
             "live_tests::real_codex_source_migration_without_model")
MARKER = "isolated Codex source migration without credentials or model commands\n"
CASES = {"modified_rev3_source", "mixed_cache", "unknown_source", "disabled_target"}


def isolated_environment(root, codex):
    # 不继承认证、代理、shell 启动文件或进程注入配置；不读取原生登录资料。
    environment = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL") if key in os.environ}
    environment["PATH"] = str(codex.parent) + os.pathsep + environment.get("PATH", os.defpath)
    paths = {
        "HOME": root / "home", "USERPROFILE": root / "home", "CODEX_HOME": root / "codex",
        "XDG_CONFIG_HOME": root / "home/.config", "XDG_DATA_HOME": root / "home/.local/share",
        "XDG_CACHE_HOME": root / "home/.cache", "TMPDIR": root / "tmp",
        "TMP": root / "tmp", "TEMP": root / "tmp",
    }
    for key, path in paths.items():
        path.mkdir(parents=True, mode=0o700, exist_ok=True)
        environment[key] = str(path)
    (root / "project").mkdir(mode=0o700)
    (root / "codex/config.toml").write_text('cli_auth_credentials_store = "file"\n', encoding="utf-8")
    (root / ".infinishell-codex-source-probe").write_text(MARKER, encoding="utf-8")
    environment["INFINISHELL_CODEX_SOURCE_ROOT"] = str(root)
    return environment


def verified_acceptance(exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    if any(not isinstance(event, dict) for event in events):
        return False
    expected = ["rust_source_migration_started", "production_runtime_probe", "rev3_fixture_verified",
                "install_observed", "migration_verified", "install_observed", "repeat_install_verified",
                "rejection_observed", "rejection_observed", "rejection_observed", "rejection_observed",
                "rust_source_migration_passed"]
    if [event.get("event") for event in events] != expected:
        return False
    started, probe, fixture, first, migration, repeat, repeated, *tail = events
    rejections, completed = tail[:-1], tail[-1]
    return (started.get("scope") == "production_rust_installer"
            and started.get("credentials_provided") is False and started.get("model_commands_sent") == 0
            and probe.get("succeeded") is True
            and fixture.get("initial_installation") == "controlled_exact_rev3_fixture"
            and fixture.get("source_files") == 36 and fixture.get("cache_files") == 10
            and fixture.get("trust_fixture_is_not_native_authorization") is True
            and first.get("phase") == "rev3_to_rev4" and first.get("succeeded") is True
            and all(migration.get(key) is True for key in (
                "rev4_source_and_cache_verified", "old_source_and_metadata_preserved",
                "old_cache_and_transaction_preserved", "user_configuration_and_trust_preserved",
                "disabled_orchestration_preserved"))
            and migration.get("transaction_phase") == "verified"
            and repeat.get("phase") == "repeat_rev4_install" and repeat.get("succeeded") is True
            and repeated.get("previous_recovery_material_preserved") is True
            and {event.get("case") for event in rejections} == CASES
            and all(event.get("rejected") is True and event.get("bytes_and_modes_unchanged") is True
                    and event.get("native_command_invoked") is False
                    and event.get("rev4_source_created") is False for event in rejections)
            and completed.get("passed") is True and completed.get("rejected_cases") == 4
            and completed.get("model_commands_sent") == 0
            and all(completed.get(key) is False for key in (
                "hook_authorization_performed", "native_hook_execution_verified",
                "windows_product_gate_opened", "gui_verified")))


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(4 * 1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def sanitize(text, root):
    text = text.replace(str(root), "<probe-root>").replace(str(Path.home()), "<user-home>")
    return re.sub(r"(?:sk-[A-Za-z0-9_-]{16,}|Bearer [A-Za-z0-9_.-]+)", "<redacted>", text)


def stop_group(process):
    # 失败时只声明尝试终止本运行器的 Unix 进程组，不冒充监督者的完整进程树证明。
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            pass
        try:
            return process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            continue
    raise RuntimeError("终止运行器进程组后仍未收到输出 EOF；保留现场")


def run_command(command, root, environment, timeout):
    process = subprocess.Popen(command, cwd=root / "project", env=environment, start_new_session=True,
                               text=True, encoding="utf-8", errors="replace",
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    try:
        output = process.communicate(timeout=timeout)[0]
        return process.returncode, output, False
    except subprocess.TimeoutExpired:
        output = stop_group(process)[0]
        return process.returncode, output, True
    except BaseException:
        stop_group(process)
        raise


def artifact_paths(output):
    return (output, output.with_suffix(".metadata.json"), output.with_suffix(".test-output.txt"))


def validate_inputs(args, repository):
    if sys.platform not in ("darwin", "linux"):
        raise ValueError("当前生产插件门控仅允许 macOS/Linux；Windows 不能计为通过")
    for path in (args.test_binary, args.codex):
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"需要可执行的原生文件：{path}")
    if args.codex.name != "codex":
        raise ValueError("固定 CLI 的原生可执行文件必须名为 codex，确保生产 PATH 探针选择同一文件")
    if not 30 <= args.timeout <= 900:
        raise ValueError("超时必须为 30–900 秒")
    paths = artifact_paths(args.output)
    if len(set(paths)) != len(paths):
        raise ValueError("证据、日志与元数据路径不能重叠")
    for path in paths:
        if path.is_relative_to(repository) or path in (args.test_binary, args.codex):
            raise ValueError("证据必须写到仓库和输入文件之外")
        if path.exists() or path.is_symlink():
            raise ValueError(f"拒绝覆盖已有证据：{path}")


def finish_private_root(root, accepted, metadata):
    metadata["private_root"] = str(root)
    metadata["private_root_removed"] = False
    if not accepted:
        return False
    try:
        shutil.rmtree(root)
    except OSError as error:
        metadata["cleanup_error"] = str(error)
        return False
    metadata["private_root_removed"] = True
    return True


def run(args):
    repository = Path(__file__).resolve().parents[2]
    validate_inputs(args, repository)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    evidence_path, metadata_path, output_path = artifact_paths(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-codex-source-")).resolve()
    metadata = {"schema_version": 1, "test": TEST_NAME, "scope": "production_rust_installer",
                "platform": sys.platform, "credentials_provided": False, "model_commands_sent": 0,
                "native_hook_execution_verified": False, "hook_authorization_performed": False,
                "windows_product_gate_opened": False, "app_restart_and_ui_verified": False,
                "supervised_process_tree_cleanup_verified": False, "accepted": False}
    output, accepted = "", False
    try:
        metadata["test_binary_sha256"] = digest(args.test_binary)
        metadata["codex_binary_sha256"] = digest(args.codex)
        for key, command in (("commit", ["git", "rev-parse", "HEAD"]),
                             ("worktree_status", ["git", "status", "--porcelain"])):
            result = subprocess.run(command, cwd=repository, capture_output=True, text=True, check=True)
            metadata[key] = result.stdout.strip() if key == "commit" else bool(result.stdout.strip())
        environment = isolated_environment(root, args.codex)
        environment["INFINISHELL_CODEX_SOURCE_ARTIFACT"] = str(evidence_path)
        code, version, timed_out = run_command([str(args.codex), "--version"], root, environment, 15)
        output += "$ codex --version\n" + version
        metadata["codex_version"] = version.strip()
        if timed_out or code != 0 or version.strip() != "codex-cli 0.147.0":
            raise RuntimeError("固定 Codex 0.147.0 版本检查失败；尚未启动安装器测试")
        command = [str(args.test_binary), "--exact", TEST_NAME, "--ignored", "--nocapture", "--test-threads=1"]
        code, test_output, timed_out = run_command(command, root, environment, args.timeout)
        output += test_output
        metadata.update({"exit_code": code, "timed_out": timed_out, "output_eof_observed": True})
        events = []
        if evidence_path.exists():
            events = [json.loads(line) for line in evidence_path.read_text(encoding="utf-8").splitlines() if line]
        accepted = not timed_out and verified_acceptance(code, test_output, events)
        metadata["rust_migration_acceptance_verified"] = accepted
    except Exception as error:
        metadata["error"] = f"{type(error).__name__}: {error}"
    finally:
        metadata["accepted"] = finish_private_root(root, accepted, metadata)
        with output_path.open("x", encoding="utf-8") as target:
            target.write(sanitize(output, root))
        with metadata_path.open("x", encoding="utf-8") as target:
            target.write(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"accepted": metadata["accepted"], "evidence": str(evidence_path),
                      "metadata": str(metadata_path)}, ensure_ascii=False))
    return 0 if metadata["accepted"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--codex", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--output", type=lambda value: Path(value).absolute(), required=True)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    # 只解析父目录，不跟随可能已存在的目标符号链接。
    args.output = args.output.parent.resolve() / args.output.name
    try:
        return run(args)
    except (OSError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    sys.exit(main())
