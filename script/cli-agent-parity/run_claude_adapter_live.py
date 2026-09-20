#!/usr/bin/env python3
"""用私有 Claude 配置或显式 API 环境运行真实 Rust 验收；不读取原生登录资料。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from prepare_claude_cli import (RELEASE_CATALOG, VERSION as DEFAULT_VERSION, current_platform,
                                verify_binary, verify_version)


TEST_NAME = "ai::cli_agent_runtime::claude::live_tests::real_claude_managed_lifecycle"
MARKER = "isolated Claude Rust adapter verification\n"
PROJECT_SETTINGS = {"permissions": {"defaultMode": "default", "ask": ["Write"]}}
API_ENVIRONMENT_KEYS = {"ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "ANTHROPIC_MODEL"}


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def authenticated_environment(root, config_dir, auth_home):
    # 保留登录时的私有 HOME 和系统用户/钥匙串身份；绝不重新初始化已登录配置。
    allowed = {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG", "LC_ALL"}
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    environment.update({
        "HOME": str(auth_home), "USERPROFILE": str(auth_home),
        "APPDATA": str(auth_home / "AppData/Roaming"),
        "LOCALAPPDATA": str(auth_home / "AppData/Local"),
        "XDG_CONFIG_HOME": str(auth_home / ".config"),
        "XDG_DATA_HOME": str(auth_home / ".local/share"),
        "XDG_CACHE_HOME": str(auth_home / ".cache"),
        "CLAUDE_CONFIG_DIR": str(config_dir),
        "TMPDIR": str(root / "tmp"), "TMP": str(root / "tmp"), "TEMP": str(root / "tmp"),
        "CLAUDE_CODE_ENTRYPOINT": "sdk-py", "DISABLE_AUTOUPDATER": "1", "DISABLE_UPDATES": "1",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "PYTHONUTF8": "1",
    })
    return environment


def prepare_project(root):
    project = root / "project"
    (project / ".claude").mkdir(parents=True, mode=0o700)
    (root / "tmp").mkdir(mode=0o700)
    with (root / ".infinishell-claude-live-probe").open("x", encoding="utf-8", newline="\n") as target:
        target.write(MARKER)
    # 只对新建的验收项目要求 Write 审批，不改变用户级权限或启用绕过。
    settings = project / ".claude/settings.local.json"
    with settings.open("x", encoding="utf-8", newline="\n") as target:
        json.dump(PROJECT_SETTINGS, target)
        target.write("\n")
    return settings


def load_api_environment(path):
    if path is None:
        return {}
    # 仅显式指定的文件可携带 API 认证；不寻找、读取或复制原生 CLI 的登录资料。
    if path.stat().st_size > 64 * 1024:
        raise ValueError("API 环境文件超过大小上限")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or not set(value).issubset(API_ENVIRONMENT_KEYS):
        raise ValueError("API 环境文件含有未允许的环境键")
    if any(not isinstance(item, str) or not item or any(character in item for character in "\x00\r\n")
           for item in value.values()):
        raise ValueError("API 环境值必须为非空单行字符串")
    if ("ANTHROPIC_API_KEY" in value) == ("ANTHROPIC_AUTH_TOKEN" in value):
        raise ValueError("API 环境必须且只能提供一种认证方式")
    return value


def sanitize(text, root, config_dir, auth_home, api_environment=None):
    for key in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"):
        secret = (api_environment or {}).get(key)
        if secret:
            text = text.replace(json.dumps(secret, ensure_ascii=False)[1:-1], "<redacted>")
            text = text.replace(secret, "<redacted>")
    for path, replacement in ((config_dir, "<private-claude-config>"),
                              (auth_home, "<private-auth-home>"),
                              (root, "<probe-root>"), (Path.home(), "<user-home>")):
        for value in {str(path), json.dumps(str(path), ensure_ascii=False)[1:-1]}:
            text = text.replace(value, replacement)
    return re.sub(r"(?<![A-Za-z0-9_-])(?:sk-[A-Za-z0-9_-]{16,}|Bearer [A-Za-z0-9_.-]+)",
                  "<redacted>", text)


def sanitize_event(value, redact):
    if isinstance(value, str):
        return redact(value)
    if isinstance(value, dict):
        return {redact(key): sanitize_event(item, redact) for key, item in value.items()}
    if isinstance(value, list):
        return [sanitize_event(item, redact) for item in value]
    return value


def verified_acceptance(exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    if any(event.get("event") == "acceptance_failed" for event in events):
        return False
    endings = [event for event in events if event.get("event") == "acceptance_passed"]
    if len(endings) != 1:
        return False
    ending = endings[0]
    native_id = ending.get("native_session_id")
    if (not isinstance(native_id, str) or not native_id
            or ending.get("scope") != "rust_adapter_process_restart"
            or ending.get("queued_input_verified") is not True
            or ending.get("same_turn_steering_supported") is not False
            or ending.get("app_restart_and_ui_verified") is not False
            or ending.get("parent_permission_ceiling_verified") is not False):
        return False
    for phase, expected in (("first_turn", "PARITY_ONE"), ("second_turn", "PARITY_TWO"),
                            ("approval_allow", "APPROVED"), ("approval_deny", "DENIED")):
        results = [event for event in events if event.get("event") == "turn_finished"
                   and event.get("phase") == phase]
        if len(results) != 1 or results[0].get("outcome") != "Completed" or results[0].get("output", "").strip() != expected:
            return False
    for phase, allowed, decision in (("approval_allow", True, "AllowOnce"), ("approval_deny", False, "DenyOnce")):
        approvals = [event for event in events if event.get("event") == "approval_requested"
                     and event.get("phase") == phase]
        if len(approvals) != 1 or approvals[0].get("exact_write_fixture") is not True or approvals[0].get("decision") != decision:
            return False
        if not any(event.get("event") == "file_effect_verified" and event.get("phase") == phase
                   and event.get("allowed") is allowed for event in events):
            return False
    queued = [event for event in events if event.get("event") == "queued_input_result_verified"]
    if len(queued) != 1 or queued[0].get("native_acknowledgement_verified") is not True or queued[0].get("same_turn_steering_supported") is not False:
        return False
    if not any(event.get("event") == "queued_input_submitted" and event.get("submitted_while_running") is True
               and event.get("same_turn_steering_verified") is False for event in events):
        return False
    cancellations = [event for event in events if event.get("event") == "turn_finished"
                     and event.get("phase") == "cancel"]
    resumes = [event for event in events if event.get("event") == "turn_finished"
               and event.get("phase") == "resume_result"]
    if (len(cancellations) != 1 or cancellations[0].get("outcome") != "Cancelled"
            or len(resumes) != 1 or resumes[0].get("outcome") != "Completed"
            or not queued[0].get("marker") or resumes[0].get("output", "").strip() != queued[0]["marker"]):
        return False
    results = [event for event in events if event.get("event") == "turn_finished"]
    if len(results) != 8 or any(event.get("native_session_id") != native_id for event in results):
        return False
    turn_ids = [event.get("turn_id") for event in results]
    if len(set(turn_ids)) != 8 or any(not isinstance(turn_id, str) or not turn_id for turn_id in turn_ids):
        return False
    joins = [event for event in events if event.get("event") == "input_joined"]
    matched_joins = []
    for result in results:
        if not any(event.get("event") == "message_accepted" and event.get("phase") == result.get("phase")
                   and event.get("message_id") == result["turn_id"] and event.get("turn_id") == result["turn_id"]
                   for event in events):
            return False
        starts = [event for event in events if event.get("event") == "turn_started"
                  and event.get("turn_id") == result["turn_id"]]
        input_joins = [event for event in joins if event.get("message_id") == result["turn_id"]]
        if len(starts) == 1 and not input_joins:
            continue
        if starts or len(input_joins) != 1 or result.get("phase") != "queued_input":
            return False
        joined = input_joins[0]
        execution = next((event for event in results if event.get("turn_id") == joined.get("turn_id")), None)
        execution_starts = [event for event in events if event.get("event") == "turn_started"
                            and event.get("turn_id") == joined.get("turn_id")]
        accepts = [event for event in events if event.get("event") == "message_accepted"
                   and event.get("message_id") == result["turn_id"]]
        if (execution is None or len(execution_starts) != 1 or len(accepts) != 1
                or joined.get("phase") != "queued_input" or joined.get("native_session_id") != native_id
                or execution.get("phase") != "queued_input" or execution is result
                or execution.get("outcome") != result.get("outcome")
                or execution.get("output") != result.get("output")
                or not events.index(execution_starts[0]) < events.index(joined) < events.index(result) < events.index(execution)
                or events.index(accepts[0]) >= events.index(joined)):
            return False
        matched_joins.append(joined)
    if len(matched_joins) != len(joins):
        return False
    queue_results = [event for event in results if event.get("phase") == "queued_input"]
    if len(queue_results) != 2 or any(event.get("outcome") != "Completed" for event in queue_results):
        return False
    shutdowns = [event for event in events if event.get("event") == "connection_shutdown"]
    return len(shutdowns) == 2 and all(event.get("native_session_id") == native_id for event in shutdowns)


def validate_paths(args):
    for name in ("test_binary", "claude", "supervisor", "config_dir", "auth_home", "api_environment_file"):
        path = getattr(args, name)
        if path is None:
            continue
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & 0x400:
            raise ValueError(f"{name} 不能是符号链接或重解析点")
        path = path.resolve(strict=True)
        if name in ("config_dir", "auth_home"):
            if not path.is_dir():
                raise ValueError(f"{name} 必须是现有私有目录")
            if path in (Path.home().resolve(), (Path.home() / ".claude").resolve()):
                raise ValueError("不能使用用户默认 HOME 或默认 Claude 配置进行此验收")
        elif not path.is_file():
            raise ValueError(f"{name} 必须是可执行文件")
        setattr(args, name, path)
    if args.test_binary == args.supervisor:
        raise ValueError("监督入口必须是同提交主程序或 TUI，不能使用 libtest")
    args.output = args.output.resolve()
    if args.output.suffix != ".ndjson":
        raise ValueError("证据输出必须使用 .ndjson 扩展名")
    inputs = {args.test_binary, args.claude, args.supervisor, args.api_environment_file}
    outputs = (args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".test-output.txt"))
    if len(set(outputs)) != 3:
        raise ValueError("输出须使用 .ndjson 等独立扩展名，不能与元数据或测试日志重名")
    for output in outputs:
        if output in inputs or output.is_relative_to(args.config_dir) or output.is_relative_to(args.auth_home):
            raise ValueError("证据不得覆盖输入、认证 HOME 或私有配置目录")
        if output.exists() or output.is_symlink():
            raise ValueError("证据文件已存在，请使用新名称，避免误用旧成功记录")


def run(args):
    repository = Path(__file__).resolve().parents[2]
    # 先核对固定文件及无认证版本输出，再读取显式 API 环境；旧派生运行器缺省仍使用 273。
    selected_version = getattr(args, "claude_version", DEFAULT_VERSION)
    target = current_platform()
    verified_cli = verify_binary(args.claude, target, selected_version)
    root = Path(tempfile.mkdtemp(prefix="infinishell-claude-adapter-")).resolve()
    detected = verify_version(args.claude, root, selected_version)
    if verify_binary(args.claude, target, selected_version) != verified_cli:
        raise ValueError("原生 Claude 在版本探测期间变化")
    api_environment = load_api_environment(args.api_environment_file)
    settings = prepare_project(root)
    environment = authenticated_environment(root, args.config_dir, args.auth_home)
    environment.update(api_environment)
    redact = lambda text: sanitize(text, root, args.config_dir, args.auth_home, api_environment)
    environment.update({
        "INFINISHELL_CLAUDE_LIVE_ROOT": str(root),
        "INFINISHELL_CLAUDE_LIVE_CONFIG_DIR": str(args.config_dir),
        "INFINISHELL_CLAUDE_LIVE_EXECUTABLE": str(args.claude),
        "INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION": selected_version,
        "INFINISHELL_CLAUDE_LIVE_ARTIFACT": str(args.output),
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor),
    })
    if args.model:
        environment["INFINISHELL_CLAUDE_LIVE_MODEL"] = args.model
    metadata = {
        "test": TEST_NAME, "scope": "rust_adapter_process_restart", "platform": sys.platform,
        "native_credential_files_read_or_copied": False, "private_config_supplied": True,
        "authentication_source": "explicit_api_environment" if api_environment else "existing_private_login",
        "api_environment_values_recorded": False,
        "runner_modifies_auth_configuration": False, "native_cli_may_refresh_credentials": True,
        "private_auth_home_preserved": True, "model_requests_expected": True,
        "app_restart_and_ui_verified": False, "same_turn_steering_supported": False,
        "parent_permission_ceiling_verified": False, "filesystem_sandbox_verified": False,
        "supervised_process_lifecycle": True, "private_workspace_preserved": True,
        "private_workspace": str(root), "project_settings_sha256": digest(settings),
        "test_binary_sha256": digest(args.test_binary), "cli_binary_sha256": digest(args.claude),
        "supervisor_binary_sha256": digest(args.supervisor), "acceptance_passed": False,
        "cli_version": detected, "requested_cli_version": selected_version, "cli": verified_cli,
    }
    for command, key in ((["git", "rev-parse", "HEAD"], "commit"), (["git", "status", "--porcelain"], "worktree_dirty")):
        result = subprocess.run(command, cwd=repository, text=True, capture_output=True, check=True)
        metadata[key] = bool(result.stdout.strip()) if key == "worktree_dirty" else result.stdout.strip()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    output = ""
    events = []
    owns_output = False
    try:
        # 预先创建空证据，防止零测试成功被当成验收，也拒绝复用已有文件。
        with args.output.open("x", encoding="utf-8"):
            pass
        owns_output = True
        command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        process = subprocess.Popen(command, cwd=repository, env=environment,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding="utf-8", errors="replace")
        try:
            output, _ = process.communicate(timeout=900)
        except subprocess.TimeoutExpired:
            metadata["timed_out"] = True
            process.kill()
            output, _ = process.communicate(timeout=15)
        except BaseException:
            process.kill()
            process.wait(timeout=15)
            raise
        metadata["test_exit_code"] = process.returncode
        raw_events = args.output.read_text(encoding="utf-8")
        # 逐个字符串脱敏后重新序列化，秘密含引号或反斜杠时也不会破坏 JSON 证据。
        try:
            events = [sanitize_event(json.loads(line), redact) for line in raw_events.splitlines()]
        finally:
            args.output.write_text(redact(raw_events), encoding="utf-8", newline="\n")
        args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in events),
                               encoding="utf-8", newline="\n")
        metadata["project_settings_unchanged"] = digest(settings) == metadata["project_settings_sha256"]
        metadata["cli_binary_unchanged"] = verify_binary(args.claude, target, selected_version) == verified_cli
        metadata["acceptance_passed"] = (not metadata.get("timed_out", False)
            and metadata["project_settings_unchanged"] and metadata["cli_binary_unchanged"]
            and verified_acceptance(process.returncode, output, events))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        # 不输出 subprocess 的任意 stdout/stderr，也不读取失败时可能存在的认证资料。
        metadata["runner_error"] = redact(f"{type(error).__name__}: {error}")
    finally:
        # 用户中断、无效 JSON 或子进程异常也必须清理本运行器持有的证据中的认证值。
        if owns_output and args.output.exists():
            raw = args.output.read_text(encoding="utf-8", errors="replace")
            args.output.write_text(redact(raw), encoding="utf-8", newline="\n")
        args.output.with_suffix(".test-output.txt").write_text(
            redact(output), encoding="utf-8", newline="\n")
        args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("真实 Rust Claude 适配器验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
    print(f"证据：{args.output}")
    print(f"私有工作目录已保留：{root}")
    return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True, help="所选固定官方版本的原生可执行文件")
    parser.add_argument("--claude-version", choices=tuple(RELEASE_CATALOG), default=DEFAULT_VERSION,
                        help="精确官方版本；缺省保留 2.1.273")
    parser.add_argument("--supervisor", type=Path, required=True, help="同提交的主程序或 TUI 监督入口")
    parser.add_argument("--config-dir", type=Path, required=True, help="私有 CLAUDE_CONFIG_DIR；使用已有登录或显式 API 环境")
    parser.add_argument("--auth-home", type=Path, required=True, help="登录时使用的私有 HOME/USERPROFILE")
    parser.add_argument("--api-environment-file", type=Path, help="显式私有 JSON；只读取允许的 Anthropic API 环境键")
    parser.add_argument("--model", help="可选原生模型 ID；未指定时保留 CLI 默认模型")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        validate_paths(args)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    try:
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        # 启动前的配置错误也不展开含有敏感环境值的对象。
        parser.exit(2, f"验收运行器启动失败：{type(error).__name__}，请检查显式输入文件。\n")


if __name__ == "__main__":
    raise SystemExit(main())
