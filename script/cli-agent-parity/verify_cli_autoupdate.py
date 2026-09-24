#!/usr/bin/env python3
"""准备或运行唯一的生产升级测试；默认不执行，不继承认证或用户配置。"""

import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import time


SCOPE = "cli_autoupdate_native_product"
MAX_BINARY_BYTES = 8 * 1024 ** 3
TEST_NAME = "terminal::cli_agent_updates::sources::live_tests::real_native_update_without_model"
MARKER = b"InfiniShell private updater fixture; no credentials or model inputs\n"
CHANNELS = {"codex": {"follow_installation", "latest", "alpha"},
            "claude": {"follow_installation", "latest", "stable"},
            "grok": {"follow_installation", "stable", "alpha"}}
EXPECTED = {"updated", "source_changed_rejected", "channel_only", "command_failed_rolled_back",
            "interrupted_recovered"}
PATHS = {"HOME": "home", "USERPROFILE": "home", "CODEX_HOME": "home/.codex",
         "CLAUDE_CONFIG_DIR": "home/.claude", "GROK_HOME": "home/.grok",
         "XDG_CONFIG_HOME": "home/.config", "XDG_DATA_HOME": "home/.local/share",
         "XDG_CACHE_HOME": "home/.cache", "XDG_STATE_HOME": "home/.local/state",
         "TMPDIR": "tmp", "TMP": "tmp", "TEMP": "tmp"}
AUTH_PATHS = ("home/.codex/auth.json", "home/.grok/auth.json", "home/.claude/.credentials.json",
              "home/.claude/credentials.json", "home/.netrc", "home/.npmrc")
MANIFEST_KEYS = {"schema", "scope", "case_id", "root", "agent", "channel", "expected", "entry",
                 "old_version", "target_version", "old_binary", "target_binary", "worker",
                 "supervisor", "source_manifest", "gates_report", "bundle_report", "timeout_seconds"}
FIXED_CANDIDATES = {
    "codex": ("0.156.1", "0196e89fe5a7598f816ee54232c3d7c26d75e502ab5cfe2c9240e81d90f7255a"),
    "claude": ("2.1.280", "387a5c5dcdbb815085edf0baf79591f9d8894efe922bceaf3d75b1b08055229d"),
    "grok": ("1.0.41", "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d"),
}
LINUX_FIXED_CANDIDATES = {
    "codex": ("0.156.1", "0b2e9301d6100dddda3b9d5c80ebaeaa3a2f1962388f2f36f6b96a9f08b1f33f"),
    "claude": ("2.1.280", "1e08503dbdf3c2cb0d706d32f3408277388d1c76ef108673e8fe42c1b322925b"),
    "grok": ("1.0.41", "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407"),
}

BINDINGS = ("old_binary", "target_binary", "worker", "supervisor", "source_manifest", "gates_report", "bundle_report")
REQUIRED_SOURCE_FILES = ("app/src/terminal/cli_agent_updates.rs",
    "app/src/terminal/cli_agent_updates/sources.rs",
    "app/src/terminal/cli_agent_updates/sources_live_tests.rs",
    "app/src/ai/cli_agent_runtime/managed_process.rs",
    "app/src/ai/cli_agent_runtime/managed_process_atomic_linux.rs",
    "app/src/ai/cli_agent_runtime/managed_process_atomic_linux_glibc.rs",
    "app/src/ai/cli_agent_runtime/managed_process_atomic_macos.rs",
    "app/src/ai/cli_agent_runtime/managed_process_atomic_windows.rs",
    "app/src/ai/cli_agent_runtime/codex.rs",
    "app/src/ai/cli_agent_runtime/claude.rs",
    "app/src/ai/cli_agent_runtime/grok.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/codex_source.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/codex_hook_trust.rs",
    "app/src/terminal/cli_agent_sessions/plugin_manager/notification_patch.rs",
    "script/cli-agent-parity/verify_cli_autoupdate.py")
SUPERVISOR_SOURCE_FILES = REQUIRED_SOURCE_FILES[:2] + REQUIRED_SOURCE_FILES[3:-1]
BOOLS = {"passed", "credentials_provided", "private_environment_verified", "credential_files_absent_before",
         "credential_files_absent_after", "config_bytes_unchanged", "config_semantics_unchanged",
         "config_permissions_unchanged", "old_binary_unchanged", "target_reference_unchanged",
         "entry_matches_expected", "post_version_matches", "journal_absent", "production_chain_verified",
         "same_source_build_verified", "same_commit_verified_by_runner", "config_transition_verified",
         "unrelated_config_bytes_unchanged", "config_permissions_preserved", "supervisor_generations_unchanged", "entry_unchanged",
         "failure_intent_persisted", "fixed_release_input", "test_only_target_candidate",
         "plugin_feature_flags_enabled_for_test"}
HASHES = {"manifest_sha256", "worker_sha256", "supervisor_sha256", "source_manifest_sha256", "old_sha256", "target_sha256"}
COUNTERS = {"product_inspect_calls", "product_execute_calls", "model_inputs_sent", "config_files_checked"}
EVENT_KEYS = BOOLS | HASHES | COUNTERS | {"schema", "scope", "case_id", "agent", "channel", "expected",
    "stage", "error", "failure_code", "old_version", "target_version", "config_transition", "plan_requires_native_update"}
FAILURE_CODES = {"binary_unreadable", "path_outside_fixture", "fixture_symlink", "fixture_path_unreadable",
    "invalid_agent", "invalid_channel", "unsupported_channel", "config_unreadable", "config_not_regular",
    "inspect_before_failed", "native_plan_mismatch", "native_plan_missing", "inspect_changed_configuration",
    "entry_not_symlink", "source_change_failed", "source_change_restore_conflict", "source_change_restore_failed",
    "execute_failed", "source_change_not_rejected", "invalid_expectation", "inspect_after_failed",
    "state_unavailable", "supervisor_snapshot_failed", "entry_changed_unexpectedly", "config_transition_mismatch",
    "failure_boundary_missing", "failure_boundary_invalid", "failure_boundary_read_only_failed",
    "failure_boundary_restore_failed", "native_start_timeout", "native_update_completed_before_interrupt",
    "interrupted_exit_unconfirmed", "interrupted_exit_timeout", "command_failure_not_rolled_back",
    "native_update_not_started", "failure_intent_not_persisted"}


def require(condition, code):
    if not condition:
        raise ValueError(code)


def is_hash(value):
    return type(value) is str and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def is_version(value):
    return (type(value) is str and len(value) <= 64
            and re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-alpha(?:\.(?:0|[1-9][0-9]*)){0,2})?", value) is not None)


def digest(path):
    result = hashlib.sha256()
    require(path.is_file(), "binary_not_regular")
    require(path.stat().st_size <= MAX_BINARY_BYTES, "binary_too_large")
    consumed = 0
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            consumed += len(chunk)
            require(consumed <= MAX_BINARY_BYTES, "binary_too_large")
            result.update(chunk)
    return result.hexdigest()


def exclusive_bytes(path, body):
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(path, flags, 0o600), "wb") as stream:
        stream.write(body)
        stream.flush()
        os.fsync(stream.fileno())


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def private_bytes(path):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= 64 * 1024, "private_file_shape")
    if os.name == "posix":
        require(stat.S_IMODE(info.st_mode) == 0o600 and info.st_uid == os.geteuid(), "private_file_permissions")
    return path.read_bytes()


def under(root, path):
    return path.is_absolute() and path != root and root in path.parents and ".." not in path.parts


def windows_file_attributes_are_plain(attributes):
    return attributes & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400) == 0


def plain_path(root, path):
    require(under(root, path), "path_outside_fixture")
    for item in (path, *path.parents):
        if item == root:
            break
        if os.path.lexists(item):
            info = item.lstat()
            require(not item.is_symlink()
                    and (os.name != "nt" or windows_file_attributes_are_plain(info.st_file_attributes)),
                    "fixture_symlink")


def auth_absent(root):
    return all(not os.path.lexists(root / relative) for relative in AUTH_PATHS)


def valid_transition(transition, agent, channel, expected):
    if transition is None:
        return expected != "channel_only"
    if type(transition) is not dict or expected not in {"updated", "channel_only"}:
        return False
    if transition.get("kind") == "channel":
        allowed = {"grok": ("stable", "alpha"), "claude": ("latest", "stable")}.get(agent)
        if not allowed or set(transition) != {"kind", "before", "after"}:
            return False
        before, after = transition["before"], transition["after"]
        return (type(after) is str and after == channel and after in allowed
                and (before is None or type(before) is str and before in allowed)
                and (allowed[0] if before is None else before) != after)
    if transition.get("kind") == "codex_marker":
        return (agent == "codex" and expected == "updated"
                and set(transition) == {"kind", "before_sha256", "after_sha256"}
                and all(transition[key] is None or is_hash(transition[key]) for key in ("before_sha256", "after_sha256")))
    return False


def validate_manifest(value):
    require(type(value) is dict and MANIFEST_KEYS <= set(value)
            <= MANIFEST_KEYS | {"config_transition", "fixed_release_input", "test_only_target_candidate"}, "manifest_fields")
    require(all(type(value.get(key, False)) is bool for key in ("fixed_release_input", "test_only_target_candidate")),
            "fixed_release_flags")
    require(type(value["schema"]) is int and value["schema"] == 1 and value["scope"] == SCOPE, "manifest_schema")
    require(type(value["case_id"]) is str and re.fullmatch(r"[a-z0-9-]{1,64}", value["case_id"]) is not None, "case_id")
    require(type(value["agent"]) is str and value["agent"] in CHANNELS, "agent")
    require(type(value["channel"]) is str and value["channel"] in CHANNELS[value["agent"]], "channel")
    require(type(value["expected"]) is str and value["expected"] in EXPECTED, "expectation")
    require(is_version(value["old_version"]) and is_version(value["target_version"])
            and (value["old_version"] == value["target_version"]) == (value["expected"] == "channel_only"), "versions")
    require(type(value["timeout_seconds"]) is int and 30 <= value["timeout_seconds"] <= 600, "deadline")
    for name in ("root", "entry"):
        require(type(value[name]) is str and Path(value[name]).is_absolute(), "absolute_path")
    for name in BINDINGS:
        row = value[name]
        require(type(row) is dict and set(row) == {"path", "sha256"}
                and type(row["path"]) is str and Path(row["path"]).is_absolute() and is_hash(row["sha256"]), "binary_binding")
    if value.get("test_only_target_candidate", False):
        candidates = FIXED_CANDIDATES if sys.platform == "darwin" else (
            LINUX_FIXED_CANDIDATES if sys.platform == "linux" and platform.machine().lower() in ("x86_64", "amd64") else {})
        require(value.get("fixed_release_input", False)
                and (value["target_version"], value["target_binary"]["sha256"]) == candidates.get(value["agent"]),
                "fixed_candidate_binding")
    if value["expected"] == "channel_only":
        require(value["old_binary"] == value["target_binary"], "channel_only_binary_binding")
    else:
        require(value["old_binary"]["path"] != value["target_binary"]["path"]
                and value["old_binary"]["sha256"] != value["target_binary"]["sha256"], "old_and_target_must_differ")
    require(valid_transition(value.get("config_transition"), value["agent"], value["channel"], value["expected"]),
            "config_transition")
    return value


def verify_source_manifest(source):
    files = source.get("files")
    require(type(files) is list, "source_file_manifest_missing")
    root = Path(__file__).resolve().parents[2]
    seen = set()
    for row in files:
        require(type(row) is dict and set(row).issubset({"path", "sha256", "bytes"})
                and set(row) >= {"path", "sha256"}, "source_file_manifest_invalid")
        relative = row["path"]
        require(type(relative) is str and relative not in seen and not Path(relative).is_absolute()
                and all(part not in ("", ".", "..") for part in Path(relative).parts)
                and is_hash(row["sha256"]), "source_file_manifest_invalid")
        seen.add(relative)
        path = root / relative
        info = path.lstat()
        require(stat.S_ISREG(info.st_mode) and not path.is_symlink()
                and path.resolve(strict=True).is_relative_to(root)
                and digest(path) == row["sha256"]
                and ("bytes" not in row or type(row["bytes"]) is int and row["bytes"] == info.st_size),
                "source_file_manifest_mismatch")
    require(set(REQUIRED_SOURCE_FILES).issubset(seen), "source_file_manifest_missing")


def strict_signature_verified(path):
    if sys.platform != "darwin":
        return True
    result = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(path)],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        timeout=30, check=False)
    return result.returncode == 0


def binary_contains_all(path, needles):
    maximum = max((len(needle) for needle in needles), default=0)
    require(maximum > 0 and all(needles), "source_build_binding_invalid")
    found = [False] * len(needles)
    tail = b""
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            body = tail + chunk
            found = [present or needle in body for present, needle in zip(found, needles)]
            if all(found):
                return True
            tail = body[-(maximum - 1):] if maximum > 1 else b""
    return all(found)


def supervisor_source_binding_verified(value):
    root = Path(__file__).resolve().parents[2]
    sources = [(root / relative).read_bytes() for relative in SUPERVISOR_SOURCE_FILES]
    return binary_contains_all(Path(value["supervisor"]["path"]), sources)


def verify_build_binding(value):
    records = {}
    for name in ("source_manifest", "gates_report", "bundle_report"):
        row = value[name]
        path = Path(row["path"])
        require(path.stat().st_size <= 1024 * 1024 and digest(path) == row["sha256"], "build_binding_digest")
        records[name] = json.loads(path.read_bytes())
        require(type(records[name]) is dict, "build_binding_shape")
    source, gates, bundle = (records[name] for name in ("source_manifest", "gates_report", "bundle_report"))
    verify_source_manifest(source)
    for report in (gates, bundle):
        require(report.get("source_manifest_sha256") == value["source_manifest"]["sha256"], "source_build_mismatch")
    require(supervisor_source_binding_verified(value), "source_build_mismatch")
    require(strict_signature_verified(Path(value["supervisor"]["path"])), "supervisor_signature_not_verified")
    for report, key, name in ((gates, "test_binary", "worker"), (bundle, "worker", "supervisor")):
        actual = report.get(key)
        require(type(actual) is dict and all(actual.get(field) == value[name][field] for field in ("path", "sha256")),
                "source_worker_mismatch")


def verify_fixture(value, require_marker=True):
    validate_manifest(value)
    root = Path(value["root"])
    info = root.lstat()
    require(root.resolve(strict=True) == root and root.name.startswith("infinishell-cli-autoupdate-")
            and stat.S_ISDIR(info.st_mode), "fixture_identity")
    if os.name == "posix":
        require(stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid == os.geteuid(), "fixture_permissions")
    if require_marker:
        require(private_bytes(root / ".infinishell-cli-autoupdate") == MARKER, "fixture_marker")
    for name in ("old_binary", "target_binary"):
        path = Path(value[name]["path"])
        plain_path(root, path)
        require(path.resolve(strict=True) == path, "binary_not_canonical")
    entry = Path(value["entry"])
    plain_path(root, entry.parent)
    require(under(root, entry) and entry.is_symlink()
            and entry.resolve(strict=True) == Path(value["old_binary"]["path"]), "old_entry_binding")
    for name in BINDINGS:
        require(digest(Path(value[name]["path"])) == value[name]["sha256"], "input_digest_mismatch")
    verify_build_binding(value)
    require(auth_absent(root), "credential_file_present")
    return root


def isolated_environment(root, value):
    environment = {name: str(root / relative) for name, relative in PATHS.items()}
    for relative in set(PATHS.values()) | {"home/.local/bin", "home/.grok/bin", "project"}:
        path = root / relative
        plain_path(root, path)
        path.mkdir(parents=True, mode=0o700, exist_ok=True)
        require(path.resolve(strict=True) == path, "environment_path")
    environment.update(PATH=f"{root / 'home/.local/bin'}:{root / 'home/.grok/bin'}:/usr/bin:/bin:/usr/sbin:/sbin",
        LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8", SHELL="/bin/sh", DISABLE_AUTOUPDATER="1",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", INFINISHELL_CLI_AUTOUPDATE_ALLOW="native-update-no-model",
        INFINISHELL_CLI_AUTOUPDATE_CASE=value["case_id"], INFINISHELL_CLI_AUTOUPDATE_MANIFEST=str(root / "manifest.private.json"),
        INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=value["supervisor"]["path"])
    return environment


def prepare(args):
    value = {"schema": 1, "scope": SCOPE, "case_id": args.case_id, "root": str(args.root),
        "agent": args.agent, "channel": args.channel, "expected": args.expected,
        "old_version": args.old_version, "target_version": args.target_version, "entry": str(args.entry),
        "timeout_seconds": args.timeout}
    for name in BINDINGS:
        value[name] = {"path": str(getattr(args, name)), "sha256": getattr(args, name + "_sha256")}
    if getattr(args, "config_transition", None) is not None:
        value["config_transition"] = json.loads(args.config_transition)
    for key in ("fixed_release_input", "test_only_target_candidate"):
        if getattr(args, key, False):
            value[key] = True
    root = verify_fixture(value, require_marker=False)
    isolated_environment(root, value)
    exclusive_bytes(root / ".infinishell-cli-autoupdate", MARKER)
    exclusive_bytes(root / "manifest.private.json", encoded(value))
    return {"prepared": True, "scope": SCOPE, "case_id": value["case_id"], "native_updates_executed": 0,
            "manifest_sha256": digest(root / "manifest.private.json")}


def valid_supervised_stderr(value):
    if value is None:
        return True
    if type(value) is not dict or set(value) != {
            "status", "file_bytes", "captured_bytes", "captured_sha256", "categories", "error_codes"}:
        return False
    if type(value["status"]) is not str or value["status"] not in {"complete", "truncated", "missing", "invalid"}:
        return False
    categories = {"permission", "path", "execution", "network", "network_tls", "storage",
                  "target_integrity", "resource", "platform", "unknown", "loader_policy", "identity"}
    codes = {"EACCES", "EPERM", "ENOENT", "ENOTDIR", "ENOEXEC", "ETXTBSY", "EINVAL",
             "ECONNREFUSED", "ECONNRESET", "ECONNABORTED", "ENETUNREACH", "EHOSTUNREACH",
             "ENOTFOUND", "EAI_AGAIN", "ETIMEDOUT", "CERT_HAS_EXPIRED", "DEPTH_ZERO_SELF_SIGNED_CERT",
             "UNABLE_TO_VERIFY_LEAF_SIGNATURE", "ERR_TLS_CERT_ALTNAME_INVALID", "ENOSPC", "EDQUOT",
             "EROFS", "EBADMSG", "EILSEQ", "ENOMEM",
             "managed_process.linux_atomic_dependency_closure_unbound",
             "managed_process.linux_atomic_dependency_closure_invalid",
             "managed_process.linux_atomic_not_elf", "managed_process.linux_atomic_program_table_invalid",
             "managed_process.linux_atomic_dynamic_table_invalid", "managed_process.linux_atomic_executable_stack",
             "managed_process.linux_atomic_identity_invalid", "managed_process.linux_atomic_source_changed",
             "managed_process.linux_atomic_source_too_large", "managed_process.linux_atomic_snapshot_changed",
             "managed_process.linux_atomic_seal_incomplete", "managed_process.linux_atomic_argument_invalid",
             "managed_process.linux_atomic_environment_invalid",
             "managed_process.linux_glibc_binding_changed",
             "managed_process.linux_glibc_cache_baseline_missing",
             "managed_process.linux_glibc_cache_invalid",
             "managed_process.linux_glibc_elf_invalid",
             "managed_process.linux_glibc_format_invalid",
             "managed_process.linux_glibc_preload_present",
             "managed_process.linux_glibc_soname_mismatch",
             "managed_process.linux_glibc_system_file_untrusted"}
    for key, allowed in (("categories", categories), ("error_codes", codes)):
        if (type(value[key]) is not list or len(value[key]) > len(allowed)
                or any(type(item) is not str or item not in allowed for item in value[key])
                or len(set(value[key])) != len(value[key])):
            return False
    if type(value["captured_bytes"]) is not int or not 0 <= value["captured_bytes"] <= 65536:
        return False
    if value["status"] in {"missing", "invalid"}:
        return (value["file_bytes"] is None and value["captured_bytes"] == 0
                and value["captured_sha256"] is None and not value["categories"] and not value["error_codes"])
    return (type(value["file_bytes"]) is int and 0 <= value["file_bytes"] < 2 ** 64
            and is_hash(value["captured_sha256"]) and value["captured_bytes"] == min(value["file_bytes"], 65536)
            and (value["status"] == "truncated") == (value["file_bytes"] > 65536))


def valid_supervised_exit(value):
    if value is None:
        return True
    keys = {"status", "exit_code", "exit_reason", "containment", "cleanup_confirmed", "os_error"}
    if type(value) is not dict or not keys <= set(value) <= keys | {"stderr", "stdout"}:
        return False
    if not all(valid_supervised_stderr(value.get(stream)) for stream in ("stderr", "stdout")):
        return False
    if type(value["status"]) is not str or value["status"] not in {
            "confirmed", "receipt_missing", "receipt_invalid", "generation_unavailable",
            "generation_missing", "generation_ambiguous", "generation_invalid"}:
        return False
    if any(value[key] is not None and (type(value[key]) is not int or not -(2 ** 31) <= value[key] < 2 ** 31)
           for key in ("exit_code", "os_error")):
        return False
    if value["status"] == "confirmed":
        return (value["cleanup_confirmed"] is True and value["os_error"] is None
            and type(value["exit_reason"]) is str
            and value["exit_reason"] in {"native_exit", "stop_requested", "host_disconnected", "stdio_closed"}
            and type(value["containment"]) is str
            and value["containment"] in {"not_started", "linux_subtree", "unix_process_group", "macos_resource_coalition"})
    return (value["cleanup_confirmed"] is False
        and all(value[key] is None for key in ("exit_code", "exit_reason", "containment"))
        and value.get("stderr") is None
        and value.get("stdout") is None
        and (value["status"] == "receipt_invalid" or value["os_error"] is None))


def valid_event(event):
    if type(event) is not dict or set(event) not in (EVENT_KEYS, EVENT_KEYS | {"supervised_exit"}):
        return False
    if not valid_supervised_exit(event.get("supervised_exit")):
        return False
    if not all(type(event[key]) is bool for key in BOOLS) or not all(is_hash(event[key]) for key in HASHES):
        return False
    if not all(type(event[key]) is int and 0 <= event[key] <= 64 for key in COUNTERS):
        return False
    if event["plan_requires_native_update"] is not None and type(event["plan_requires_native_update"]) is not bool:
        return False
    return (type(event["schema"]) is int and event["schema"] == 1 and event["scope"] == SCOPE
        and type(event["case_id"]) is str and re.fullmatch(r"[a-z0-9-]{1,64}", event["case_id"]) is not None
        and type(event["agent"]) is str and event["agent"] in CHANNELS
        and type(event["channel"]) is str and event["channel"] in CHANNELS[event["agent"]]
        and type(event["expected"]) is str and event["expected"] in EXPECTED
        and type(event["stage"]) is str and event["stage"] in {"started", "inspect_before", "execute", "inspect_after", "finished"}
        and (event["error"] is None or type(event["error"]) is str and event["error"] in {
            "NotInstalled", "UnsupportedSource", "UnsupportedPlatform", "SourceChanged", "Network", "InvalidRelease",
            "ProbeFailed", "PermissionDenied", "CommandFailed", "TimedOut", "VersionMismatch", "ChannelMismatch",
            "RecoveryRequired", "PersistenceFailed"})
        and (event["failure_code"] is None or type(event["failure_code"]) is str and event["failure_code"] in FAILURE_CODES)
        and is_version(event["old_version"]) and is_version(event["target_version"])
        and valid_transition(event["config_transition"], event["agent"], event["channel"], event["expected"]))


def acceptance(exit_code, text, event, value, manifest_hash):
    if not valid_event(event) or type(exit_code) is not int or exit_code != 0:
        return False
    if not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", text):
        return False
    false_flags = {"credentials_provided", "same_commit_verified_by_runner"}
    measured_flags = {"config_bytes_unchanged", "config_semantics_unchanged", "config_permissions_unchanged",
                      "fixed_release_input", "test_only_target_candidate",
                      "supervisor_generations_unchanged", "entry_unchanged"}
    if any(event[key] is not value.get(key, False) for key in ("fixed_release_input", "test_only_target_candidate")):
        return False
    transition = value.get("config_transition")
    if event["config_transition"] != transition or event["plan_requires_native_update"] is not (value["expected"] != "channel_only"):
        return False
    if transition is None and not all(event[key] for key in measured_flags if key.startswith("config_")):
        return False
    if value["expected"] in {"channel_only", "source_changed_rejected"} and not (
            event["entry_unchanged"] and event["supervisor_generations_unchanged"]):
        return False
    if value["expected"] in {"command_failed_rolled_back", "interrupted_recovered"} and not (
            event["entry_unchanged"] and not event["supervisor_generations_unchanged"]):
        return False
    expected_error = {"source_changed_rejected": "SourceChanged",
                      "command_failed_rolled_back": "CommandFailed"}.get(value["expected"])
    return (all(event[key] is True for key in BOOLS - false_flags - measured_flags) and all(event[key] is False for key in false_flags)
        and event["stage"] == "finished" and event["failure_code"] is None
        and event["product_inspect_calls"] == 2 and event["product_execute_calls"] == 1
        and event["model_inputs_sent"] == 0 and event["config_files_checked"] == 13
        and all(event[key] == value[key] for key in ("case_id", "agent", "channel", "expected", "old_version", "target_version"))
        and event["manifest_sha256"] == manifest_hash and event["worker_sha256"] == value["worker"]["sha256"]
        and event["supervisor_sha256"] == value["supervisor"]["sha256"]
        and event["source_manifest_sha256"] == value["source_manifest"]["sha256"]
        and event["old_sha256"] == value["old_binary"]["sha256"] and event["target_sha256"] == value["target_binary"]["sha256"]
        and event["error"] == expected_error)


def group_exists(pid):
    try:
        os.killpg(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        # 无权限不能证明进程组已消失；继续按仍存在处理。
        return True


def stop_group(process):
    # 只结束本驱动独立建立的进程组；任何残留或超时都不能算通过。
    if not group_exists(process.pid):
        return True
    os.killpg(process.pid, signal.SIGTERM)
    deadline = time.monotonic() + 3
    while group_exists(process.pid) and time.monotonic() < deadline:
        process.poll()
        time.sleep(0.05)
    if group_exists(process.pid):
        os.killpg(process.pid, signal.SIGKILL)
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        return False
    return not group_exists(process.pid)


def native_platform_supported():
    return os.name == "posix"


def run(args):
    require(native_platform_supported(), "native_fixture_requires_posix")
    require(args.allow_native_update, "explicit_update_authorization_required")
    raw_manifest = private_bytes(args.manifest)
    value = validate_manifest(json.loads(raw_manifest))
    root = verify_fixture(value)
    require(args.manifest == root / "manifest.private.json" and args.case_id == value["case_id"], "case_binding")
    manifest_hash = hashlib.sha256(raw_manifest).hexdigest()
    environment = isolated_environment(root, value)
    # 先独占登记；失败预约不能被重新运行静默覆盖。
    invocation = {"scope": SCOPE, "case_id": value["case_id"], "test_name": TEST_NAME,
        "manifest_sha256": manifest_hash, "worker_sha256": value["worker"]["sha256"],
        "supervisor_sha256": value["supervisor"]["sha256"],
        "source_manifest_sha256": value["source_manifest"]["sha256"], "timeout_seconds": value["timeout_seconds"],
        "max_product_execute_calls": 1,
        "native_update_expected": value["expected"] in {"updated", "command_failed_rolled_back", "interrupted_recovered"},
        "config_transition": value.get("config_transition"), "model_inputs_sent": 0, "credentials_provided": False}
    exclusive_bytes(root / "invocation.safe.json", encoded(invocation))
    metadata = dict(invocation, passed=False, timed_out=False, test_exit_code=None,
        worker_group_initially_empty=False, worker_group_stopped=False,
        safe_event_valid=False, runner_error_type=None)
    started = time.monotonic()
    process = None
    try:
        with os.fdopen(os.open(root / "test-output.private.txt", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as log:
            process = subprocess.Popen([value["worker"]["path"], TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                cwd=root / "project", env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True)
            try:
                process.wait(timeout=value["timeout_seconds"])
            except subprocess.TimeoutExpired:
                metadata["timed_out"] = True
            metadata["test_exit_code"] = process.returncode
        metadata["worker_group_initially_empty"] = not group_exists(process.pid)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        metadata["runner_error_type"] = type(error).__name__
    finally:
        if process is not None:
            try:
                metadata["worker_group_stopped"] = stop_group(process)
            except OSError as error:
                metadata["runner_error_type"] = type(error).__name__
        try:
            event = json.loads(private_bytes(root / "events.safe.json"))
            metadata["safe_event_valid"] = valid_event(event)
            log = (root / "test-output.private.txt").read_bytes()
            require(len(log) <= 1024 * 1024, "test_log_budget")
            metadata["passed"] = (metadata["runner_error_type"] is None and not metadata["timed_out"]
                and metadata["worker_group_initially_empty"] and metadata["worker_group_stopped"]
                and acceptance(metadata["test_exit_code"], log.decode("utf-8", errors="replace"), event, value, manifest_hash)
                and auth_absent(root) and digest(Path(value["worker"]["path"])) == value["worker"]["sha256"]
                and digest(Path(value["supervisor"]["path"])) == value["supervisor"]["sha256"]
                and digest(args.manifest) == manifest_hash)
            if metadata["safe_event_valid"]:
                metadata["event"] = event
        except (OSError, ValueError) as error:
            metadata["passed"] = False
            metadata["runner_error_type"] = type(error).__name__
        metadata["elapsed_seconds"] = round(time.monotonic() - started, 3)
        exclusive_bytes(root / "receipt.safe.json", encoded(metadata))
    return metadata


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prepared = sub.add_parser("prepare", help="核对已有私有原生安装并独占写入清单；不运行 CLI")
    for name in ("root", "entry", *(name.replace("_", "-") for name in BINDINGS)):
        prepared.add_argument("--" + name, type=Path, required=True)
    for name in (name.replace("_", "-") for name in BINDINGS):
        prepared.add_argument("--" + name + "-sha256", required=True)
    for name in ("case-id", "old-version", "target-version"):
        prepared.add_argument("--" + name, required=True)
    prepared.add_argument("--agent", choices=sorted(CHANNELS), required=True)
    prepared.add_argument("--channel", choices=sorted(set.union(*CHANNELS.values())), required=True)
    prepared.add_argument("--expected", choices=sorted(EXPECTED), default="updated")
    prepared.add_argument("--timeout", type=int, default=480)
    prepared.add_argument("--config-transition", help="明确允许的单字段/标记变更 JSON；缺省要求配置字节不变")
    prepared.add_argument("--fixed-release-input", action="store_true", help="使用清单固定版本作为发行输入，不声称实时 latest")
    prepared.add_argument("--test-only-target-candidate", action="store_true", help="仅 Mac 或 Linux x64 固定版本及官方摘要的原子升级候选，不开放生产门禁")
    executed = sub.add_parser("run", help="显式授权后仅运行该清单的一次生产测试")
    executed.add_argument("--manifest", type=Path, required=True)
    executed.add_argument("--case-id", required=True)
    executed.add_argument("--allow-native-update", action="store_true")
    args = parser.parse_args(argv)
    try:
        result = prepare(args) if args.command == "prepare" else run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        known = {"manifest_fields", "manifest_schema", "case_id", "agent", "channel", "expectation", "versions", "deadline",
            "absolute_path", "binary_binding", "old_and_target_must_differ", "binary_not_regular", "binary_too_large", "private_file_shape",
            "private_file_permissions", "path_outside_fixture", "fixture_symlink", "fixture_identity", "fixture_permissions",
            "fixture_marker", "binary_not_canonical", "old_entry_binding", "input_digest_mismatch", "credential_file_present",
            "environment_path", "native_fixture_requires_posix", "explicit_update_authorization_required", "case_binding",
            "test_log_budget", "build_binding_digest", "build_binding_shape", "source_file_manifest_missing",
            "source_file_manifest_invalid", "source_file_manifest_mismatch", "source_build_binding_invalid", "source_build_mismatch", "supervisor_signature_not_verified",
            "source_worker_mismatch", "config_transition", "channel_only_binary_binding"}
        code = str(error) if type(error) is ValueError and str(error) in known else None
        print(json.dumps({"passed": False, "runner_error_type": type(error).__name__, "failure_code": code}), file=sys.stderr)
        return 2
    print(json.dumps({key: result[key] for key in ("scope", "case_id")}
                     | {"prepared": args.command == "prepare", "passed": result.get("passed", False)}, ensure_ascii=False))
    return 0 if args.command == "prepare" or result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
