#!/usr/bin/env python3
"""固定单用例的直接 libtest 退出诊断；不替代 nextest 或关闭原失败。"""

import argparse
import datetime
import hashlib
import json
import ntpath
import os
from pathlib import Path
import platform
import stat
import struct
import subprocess
import sys
import time
import uuid


BASE_SOURCE = "f4dfff885e797fd4db8be9e02f48233d33b35844"
EXPECTED_PARENT = "cf45ac9083f8df9fdcccac713526ce7c74e55414"
TEST_NAME = "workspace::view::tests::restore_conversation_in_active_pane_enters_existing_live_conversation_without_loading"
ALLOWED_SOURCE_PATHS = frozenset({
    ".github/workflows/cross-platform-preflight.yml",
    "script/ci/run_windows_exit_diagnostic.py",
    "script/ci/build_windows_exit_observer.ps1",
    "script/ci/windows_exit_observer.cpp",
    "script/ci/windows_exit_diagnostic_workflow_tests.py",
    "script/ci/windows_exit_diagnostic_tests.py",
})
NEXTEST_LIST_COMMAND = [
    "cargo", "nextest", "list", "--locked", "--workspace",
    "--exclude", "command-signatures-v2", "--exclude", "integration",
    "--exclude", "warp_tui", "--list-type", "binaries-only", "--message-format", "json",
]
METADATA_COMMAND = ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]
MAX_ATTEMPTS = 40
EXPERIMENT_BUDGET_SECONDS = 25 * 60
MSF_MAGIC = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\x00\x00\x00"


class DiagnosticError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(condition, code):
    if not condition:
        raise DiagnosticError(code)


def read_exact(stream, offset, count):
    require(0 <= offset and 0 <= count <= 64 * 1024 * 1024, "invalid_binary_range")
    stream.seek(offset)
    data = stream.read(count)
    require(len(data) == count, "truncated_binary_structure")
    return data


def read_pe_codeview(path):
    # 只读取 PE 头、节表和 RSDS，不把大型测试二进制整体加载进内存。
    with Path(path).open("rb") as stream:
        dos = read_exact(stream, 0, 64)
        require(dos[:2] == b"MZ", "invalid_pe_dos_signature")
        pe_offset = struct.unpack_from("<I", dos, 60)[0]
        header = read_exact(stream, pe_offset, 24)
        require(header[:4] == b"PE\0\0", "invalid_pe_signature")
        machine, section_count = struct.unpack_from("<HH", header, 4)
        optional_size = struct.unpack_from("<H", header, 20)[0]
        require(machine == 0x8664 and 0 < section_count <= 96, "unsupported_pe_machine")
        optional = read_exact(stream, pe_offset + 24, optional_size)
        require(len(optional) >= 168 and struct.unpack_from("<H", optional)[0] == 0x20B,
                "invalid_pe64_optional_header")
        require(struct.unpack_from("<I", optional, 108)[0] > 6, "missing_pe_debug_directory")
        debug_rva, debug_size = struct.unpack_from("<II", optional, 160)
        require(debug_rva and debug_size and debug_size % 28 == 0 and debug_size <= 28 * 128,
                "invalid_pe_debug_directory")
        sections = read_exact(stream, pe_offset + 24 + optional_size, section_count * 40)
        offsets = []
        for index in range(section_count):
            _, virtual_address, raw_size, raw_offset = struct.unpack_from("<IIII", sections, index * 40 + 8)
            delta = debug_rva - virtual_address
            if 0 <= delta and delta + debug_size <= raw_size:
                offsets.append(raw_offset + delta)
        require(len(offsets) == 1, "ambiguous_pe_debug_mapping")
        entries = read_exact(stream, offsets[0], debug_size)
        identities = []
        for offset in range(0, debug_size, 28):
            kind, size, _, pointer = struct.unpack_from("<IIII", entries, offset + 12)
            if kind != 2:
                continue
            require(25 <= size <= 65536, "invalid_codeview_size")
            record = read_exact(stream, pointer, size)
            require(record[:4] == b"RSDS" and b"\0" in record[24:], "unsupported_codeview_record")
            name = ntpath.basename(record[24:].split(b"\0", 1)[0].decode("utf-8", errors="strict"))
            require(name.lower().endswith(".pdb") and name not in (".pdb", ".."), "invalid_codeview_pdb_name")
            identities.append({"guid": str(uuid.UUID(bytes_le=record[4:20])),
                               "age": struct.unpack_from("<I", record, 20)[0], "pdb_name": name})
        require(len(identities) == 1, "ambiguous_codeview_identity")
        return identities[0]


def read_pdb_identity(path):
    # MSF7 的目录和 stream 1 均按页 seek；不 read_bytes 整个 PDB。
    with Path(path).open("rb") as stream:
        file_size = os.fstat(stream.fileno()).st_size
        header = read_exact(stream, 0, 56)
        require(header[:32] == MSF_MAGIC, "unsupported_pdb_format")
        block_size, _, blocks, directory_size, _, block_map = struct.unpack_from("<6I", header, 32)
        require(512 <= block_size <= 65536 and block_size & (block_size - 1) == 0,
                "invalid_pdb_block_size")
        require(blocks * block_size <= file_size and block_map < blocks and directory_size >= 12,
                "invalid_pdb_superblock")
        directory_blocks = (directory_size + block_size - 1) // block_size
        require(directory_size <= 64 * 1024 * 1024 and directory_blocks * 4 <= block_size,
                "unsupported_pdb_directory_map")
        block_indices = struct.unpack("<" + "I" * directory_blocks,
                                      read_exact(stream, block_map * block_size, directory_blocks * 4))
        require(all(index < blocks for index in block_indices), "invalid_pdb_directory_block")
        directory = b"".join(read_exact(stream, index * block_size, block_size)
                             for index in block_indices)[:directory_size]
        stream_count = struct.unpack_from("<I", directory)[0]
        require(2 <= stream_count <= (len(directory) - 4) // 4, "invalid_pdb_stream_count")
        sizes = struct.unpack_from("<" + "I" * stream_count, directory, 4)
        cursor = 4 + stream_count * 4
        stream_one = None
        for index, size in enumerate(sizes):
            count = 0 if size == 0xFFFFFFFF else (size + block_size - 1) // block_size
            require(cursor + count * 4 <= len(directory), "invalid_pdb_stream_directory")
            indices = struct.unpack_from("<" + "I" * count, directory, cursor)
            require(all(block < blocks for block in indices), "invalid_pdb_stream_block")
            if index == 1:
                require(size != 0xFFFFFFFF and size >= 28 and indices, "missing_pdb_info_stream")
                stream_one = read_exact(stream, indices[0] * block_size, 28)
            cursor += count * 4
        require(stream_one is not None, "missing_pdb_identity")
        return {"guid": str(uuid.UUID(bytes_le=stream_one[12:28])),
                "age": struct.unpack_from("<I", stream_one, 8)[0]}


def validate_pdb_match(codeview, pdb):
    require(codeview["guid"] == pdb["guid"] and codeview["age"] == pdb["age"], "pdb_codeview_mismatch")
    return True


def select_warp_binary(metadata, binaries):
    workspace = Path(metadata["workspace_root"])
    target = Path(metadata["target_directory"])
    require(workspace.is_absolute() and target.is_absolute(), "relative_cargo_directory")
    packages = [package for package in metadata["packages"]
                if package["name"] == "warp"
                and Path(package["manifest_path"]) == workspace / "app" / "Cargo.toml"]
    require(len(packages) == 1, "ambiguous_warp_package")
    package = packages[0]
    require(Path(binaries["rust-build-meta"]["target-directory"]) == target, "target_directory_mismatch")
    candidates = [(key, value) for key, value in binaries["rust-binaries"].items()
                  if value["package-id"] == package["id"] and value["binary-name"] == "warp"
                  and value["kind"] == "lib"]
    require(len(candidates) == 1, "ambiguous_warp_libtest")
    binary_id, candidate = candidates[0]
    path = Path(candidate["binary-path"])
    require(path.is_absolute() and path.suffix.lower() == ".exe"
            and path.is_relative_to(target), "invalid_warp_binary_path")
    require(candidate.get("binary-id", binary_id) == binary_id, "binary_id_mismatch")
    return {"package_id": package["id"], "binary_id": binary_id, "binary_path": str(path),
            "manifest_path": package["manifest_path"], "working_directory": str(workspace / "app"),
            "target_directory": str(target)}


def validate_observer_summary(summary, mode, exit_code, stop_exists):
    require(mode in ("observe", "protocol-normal", "protocol-nonzero", "protocol-sample",
                     "protocol-job-delayed", "protocol-job-live"), "invalid_observer_mode")
    require(summary["mode"] == mode and summary["observer_exit_code"] == exit_code, "observer_exit_identity_mismatch")
    for key in ("stdout_eof", "stderr_eof", "libtest_success_marker", "libtest_test_ok",
                "libtest_summary_one_passed", "timed_out", "eof_timeout", "job_terminated", "job_empty",
                "eof_deadline_recorded", "job_naturally_settled", "job_saw_live_member"):
        require(type(summary[key]) is bool, "invalid_observer_boolean")
    for key in ("sample_count", "resumed_threads", "sampler_failures"):
        require(type(summary[key]) is int and summary[key] >= 0, "invalid_observer_counter")
    require(type(summary["child_pid"]) is int and summary["child_pid"] > 0, "invalid_child_identity")
    require(summary["child_exit_code"] is None or type(summary["child_exit_code"]) is int,
            "invalid_child_exit_code")
    require(summary["active_processes_final"] is None or (type(summary["active_processes_final"]) is int
            and summary["active_processes_final"] >= 0), "invalid_final_job_count")
    if mode == "protocol-job-live":
        # 负协议只有真实拒绝持久后代才通过；不能把强制清理算作自然退出。
        require(exit_code == 126 and summary["child_exit_code"] == 0
                and summary["observer_error"] == "job_natural_accounting_not_settled"
                and summary["stop_reason"] == "observer_error" and stop_exists
                and summary["job_terminated"] and summary["job_empty"]
                and summary["active_processes_final"] == 0 and not summary["job_naturally_settled"]
                and summary["job_saw_live_member"] and summary["eof_deadline_recorded"]
                and summary["stdout_eof"] and summary["stderr_eof"]
                and not summary["timed_out"] and not summary["eof_timeout"]
                and summary["sample_count"] == summary["sampler_failures"] == 0
                and all(summary[key] for key in ("libtest_success_marker", "libtest_test_ok",
                                                "libtest_summary_one_passed")),
                "protocol_live_descendant_not_rejected")
        return {"continue": True, "reason": "live_descendant_rejected"}
    if mode != "observe":
        expected = 7 if mode == "protocol-nonzero" else 0
        require(exit_code == summary["child_exit_code"] == expected, "protocol_child_exit_mismatch")
        require(summary["stdout_eof"] and summary["stderr_eof"] and not summary["timed_out"]
                and not summary["eof_timeout"] and summary["observer_error"] is None
                and summary["sampler_failures"] == 0 and summary["job_empty"]
                and summary["active_processes_final"] == 0 and summary["eof_deadline_recorded"]
                and summary["job_naturally_settled"],
                "protocol_observation_failed")
        markers = [summary[key] for key in ("libtest_success_marker", "libtest_test_ok", "libtest_summary_one_passed")]
        require(not any(markers) if mode == "protocol-nonzero" else all(markers), "protocol_markers_mismatch")
        require(stop_exists == (mode in ("protocol-nonzero", "protocol-sample")), "protocol_stop_mismatch")
        require(summary["sample_count"] == (2 if mode == "protocol-sample" else 0), "protocol_sample_mismatch")
        require(not summary["job_terminated"], "protocol_required_job_termination")
        expected_reason = {"protocol-normal": None, "protocol-nonzero": "child_nonzero",
                           "protocol-sample": "alive_at_30s", "protocol-job-delayed": None}[mode]
        require(summary["stop_reason"] == expected_reason, "protocol_stop_reason_mismatch")
        if mode == "protocol-sample":
            require(summary["resumed_threads"] > 0, "protocol_resume_missing")
        if mode == "protocol-job-delayed":
            require(summary["job_saw_live_member"], "protocol_delayed_member_not_observed")
        return {"continue": True, "reason": "protocol_passed"}
    # 退出码0不能覆盖采样、EOF缺失或STOP，原首次异常永远保留。
    if stop_exists or summary["stop_reason"] is not None:
        return {"continue": False, "reason": "native_stop"}
    for condition, reason in [
        (exit_code != 0 or summary["child_exit_code"] != 0, "nonzero_or_unknown_exit"),
        (summary["observer_error"] is not None or summary["sampler_failures"] != 0, "observer_error"),
        (summary["sample_count"] != 0, "sampled_at_30_seconds"),
        (summary["timed_out"] or summary["eof_timeout"], "timeout"),
        (not summary["eof_deadline_recorded"] or not summary["stdout_eof"] or not summary["stderr_eof"], "incomplete_eof"),
        (not summary["job_empty"] or summary["active_processes_final"] != 0, "job_not_empty_or_unverified"),
        (summary["job_terminated"], "job_required_termination"),
        (not summary["job_naturally_settled"], "job_natural_completion_not_observed"),
        (not all(summary[key] for key in ("libtest_success_marker", "libtest_test_ok", "libtest_summary_one_passed")),
         "exact_one_test_success_not_proven"),
    ]:
        if condition:
            return {"continue": False, "reason": reason}
    return {"continue": True, "reason": "direct_libtest_exit_observed"}


def plain_path(path):
    path = Path(path).absolute()
    for item in [*reversed(path.parents), path]:
        info = item.lstat()
        require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                "reparse_or_symbolic_path")
    return path


def file_identity(path):
    path = plain_path(path)
    before = path.stat()
    require(stat.S_ISREG(before.st_mode) and before.st_size > 0, "not_nonempty_regular_file")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    after = path.stat()
    fields = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns")
    require(all(getattr(before, field) == getattr(after, field) for field in fields), "file_changed_during_hash")
    return {"path": str(path), "sha256": digest.hexdigest(),
            **{field: getattr(after, field) for field in fields}}


def git_output(repo, *arguments):
    result = subprocess.run(["git", *arguments], cwd=repo, capture_output=True, timeout=60, check=False)
    require(result.returncode == 0, "git_identity_command_failed")
    return result.stdout.decode("utf-8", errors="strict").strip()


def source_identity(repo):
    head = git_output(repo, "rev-parse", "HEAD")
    require(git_output(repo, "rev-parse", "HEAD^") == EXPECTED_PARENT, "unexpected_source_parent")
    require(os.environ.get("GITHUB_SHA") == head, "github_source_mismatch")
    require(not git_output(repo, "status", "--porcelain=v1", "--untracked-files=all"), "worktree_not_clean")
    changed = git_output(repo, "diff", "--name-only", "--no-renames", BASE_SOURCE, "HEAD").splitlines()
    require(changed and set(changed) <= ALLOWED_SOURCE_PATHS, "product_tree_changed")
    return {"head": head, "parent": EXPECTED_PARENT, "base_source": BASE_SOURCE,
            "tree": git_output(repo, "rev-parse", "HEAD^{tree}"),
            "changed_paths": sorted(changed), "outside_diagnostic_allowlist_equal_to_f4d": True,
            "worktree_clean": True}


def environment_contract():
    # 只读取影响这次解析/运行的固定白名单；不输出全部环境或覆写内容。
    forbidden = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "CARGO_BUILD_RUSTFLAGS",
                 "RUSTC", "RUSTC_WORKSPACE_WRAPPER", "RUSTC_BOOTSTRAP", "CARGO_BUILD_RUSTC",
                 "CARGO_BUILD_TARGET_DIR", "RUST_TEST_THREADS", "RUST_TEST_TIME_UNIT", "RUST_TEST_TIME_INTEGRATION"}
    forbidden.update(key for key in os.environ if (key.startswith("CARGO_TARGET_")
                     and (key.endswith("_RUNNER") or key.endswith("_RUSTFLAGS") or key.endswith("_LINKER")))
                     or key.startswith("CARGO_PROFILE_"))
    present = sorted(key for key in forbidden if os.environ.get(key)
                     or (key in {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"} and key in os.environ))
    return {"checked_override_names": sorted(forbidden), "unexpected_nonempty_overrides": present,
            "features": [], "extra_debug_flags": [],
            "inherited": {key: os.environ.get(key) for key in (
                "CARGO_TARGET_DIR", "RUSTC_WRAPPER", "SCCACHE_DIR", "OPENSSL_SRC_PERL", "LIBCLANG_PATH", "RUST_BACKTRACE")}}


def command_to_files(command, cwd, root, name, timeout):
    with (root / (name + ".stdout.log")).open("xb") as stdout, (root / (name + ".stderr.log")).open("xb") as stderr:
        result = subprocess.run(command, cwd=cwd, stdout=stdout, stderr=stderr, timeout=timeout, check=False)
    require(result.returncode == 0, name + "_failed")
    return root / (name + ".stdout.log")


def metadata_command(command, cwd, root, name, timeout):
    # 原始 JSON 可能包含构建脚本环境；只在内存解析，归档摘要和明确白名单投影。
    with (root / (name + ".stderr.log")).open("xb") as stderr:
        result = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=stderr, timeout=timeout, check=False)
    require(result.returncode == 0, name + "_failed")
    require(len(result.stdout) <= 64 * 1024 * 1024, "metadata_output_too_large")
    return json.loads(result.stdout), {"command": command, "stdout_bytes": len(result.stdout),
                                       "stdout_sha256": hashlib.sha256(result.stdout).hexdigest()}


def observer_call(observer, binary, output, cwd, mode):
    output.mkdir()
    env = os.environ.copy()
    env.update({"INFINISHELL_EXIT_OBSERVER_BINARY": str(binary),
                "INFINISHELL_EXIT_OBSERVER_ROOT": str(output), "INFINISHELL_EXIT_OBSERVER_TEST": TEST_NAME,
                "CARGO_MANIFEST_DIR": str(cwd)})
    argv = [str(observer), "--observe"] if mode == "observe" else [str(observer), "--protocol-test", mode.removeprefix("protocol-")]
    with (output / "observer.stdout.log").open("xb") as stdout, (output / "observer.stderr.log").open("xb") as stderr:
        result = subprocess.run(argv, cwd=cwd, env=env, stdout=stdout, stderr=stderr, timeout=75, check=False)
    summaries = list(output.glob("observation-*/summary.json"))
    require(len(summaries) == 1, "missing_or_ambiguous_observer_summary")
    summary = json.loads(plain_path(summaries[0]).read_text(encoding="utf-8"))
    try:
        decision = validate_observer_summary(summary, mode, result.returncode, (output / "STOP.json").exists())
    except DiagnosticError as error:
        decision = {"continue": False, "reason": error.code}
    return {"mode": mode, "summary": str(summaries[0].relative_to(output.parent)),
            "summary_identity": file_identity(summaries[0]), "observer_exit_code": result.returncode,
            "observation": summary, "decision": decision}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--observer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(os.name == "nt", "windows_only")
    run_root = plain_path(Path(os.environ["INFINISHELL_EXIT_DIAGNOSTIC_ROOT"]))
    require(run_root.is_relative_to(plain_path(Path(os.environ["RUNNER_TEMP"]))), "output_outside_runner_temp")
    output = args.output.absolute()
    require(output == run_root / "evidence" and args.observer.absolute() == run_root / "build" / "windows-exit-observer.exe",
            "unexpected_diagnostic_paths")
    output.mkdir()
    repo = Path(__file__).absolute().parents[2]
    record = {"schema": 1, "started_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "status": "started", "scope": "direct-libtest environment diagnostic",
              "non_gating": True, "original_full_failure_closed": False, "max_attempts": MAX_ATTEMPTS,
              "no_retry": True, "sample_after_seconds": 30, "native_hard_limit_seconds": 60,
              "native_eof_observation_ms": 200, "cleanup_ready": False, "protocol_tests": [], "attempts": [],
              "test_name": TEST_NAME, "fixed_test_arguments": ["--exact", TEST_NAME, "--nocapture"],
              "metadata_command": METADATA_COMMAND, "binary_list_command": NEXTEST_LIST_COMMAND,
              "nextest_environment_equivalence_claimed": False,
              "nextest_environment_layers_not_replayed": [
                  "cargo config env", "package build-script OUT_DIR/rustc-env", "CARGO_PKG_*", "dylib PATH",
                  "NEXTEST metadata variables and nextest subprocess supervision"],
              "os": {"system": platform.system(), "release": platform.release(), "version": platform.version(),
                     "machine": platform.machine(), "python": platform.python_version()},
              "binary_or_pdb_uploaded": False, "environment_or_locals_or_dumps_collected": False}
    exit_code = 1
    try:
        record["source"] = source_identity(repo)
        record["environment_contract"] = environment_contract()
        require(not record["environment_contract"]["unexpected_nonempty_overrides"],
                "unexplained_build_or_test_environment_override")
        require(record["environment_contract"]["inherited"]["RUST_BACKTRACE"] == "1", "unexpected_rust_backtrace")
        observer = plain_path(args.observer)
        record["observer"] = file_identity(observer)
        receipt = json.loads(plain_path(run_root / "build" / "build-receipt.json").read_text(encoding="utf-8-sig"))
        require(receipt["status"] == "compiled" and receipt["source_unchanged"] is True
                and receipt["source_sha"] == record["source"]["head"]
                and receipt["executable"]["sha256"] == record["observer"]["sha256"], "observer_build_binding_mismatch")
        require(receipt["source_sha256"] == file_identity(repo / "script/ci/windows_exit_observer.cpp")["sha256"],
                "observer_source_hash_mismatch")
        require(receipt["build_script_sha256"] == file_identity(repo / "script/ci/build_windows_exit_observer.ps1")["sha256"],
                "observer_build_script_hash_mismatch")
        record["observer_build_receipt"] = file_identity(run_root / "build" / "build-receipt.json")
        record["tool_versions"] = {}
        for name, argv, expected in [("rustc", ["rustc", "--version"], "rustc 1.92.0 "),
                                     ("cargo", ["cargo", "--version"], "cargo 1.92.0 "),
                                     ("nextest", ["cargo", "nextest", "--version"], "cargo-nextest 0.9.143")]:
            path = command_to_files(argv, repo, output, "version-" + name, 60)
            version = path.read_text(encoding="utf-8").strip()
            record["tool_versions"][name] = version
            require(version.startswith(expected) if name != "nextest" else
                    (version == expected or version.startswith(expected + " ")), "unexpected_" + name + "_version")
        metadata, metadata_identity = metadata_command(METADATA_COMMAND, repo, output, "cargo-metadata", 120)
        binaries, binaries_identity = metadata_command(NEXTEST_LIST_COMMAND, repo, output, "nextest-binaries", 45 * 60)
        selected = select_warp_binary(metadata, binaries)
        record["selected"] = selected
        record["binary_list_metadata"] = binaries_identity
        record["cargo_package_metadata"] = metadata_identity
        record["rust_build_meta_field_names"] = sorted(binaries["rust-build-meta"])
        record["rust_build_meta_sha256"] = hashlib.sha256(
            json.dumps(binaries["rust-build-meta"], sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        cwd = plain_path(Path(selected["working_directory"]))
        require(cwd == repo / "app", "unexpected_libtest_working_directory")
        binary = plain_path(Path(selected["binary_path"]))
        codeview = read_pe_codeview(binary)
        pdb = plain_path(binary.parent / codeview["pdb_name"])
        pdb_identity = read_pdb_identity(pdb)
        validate_pdb_match(codeview, pdb_identity)
        bound_files = {"binary": file_identity(binary), "pdb": file_identity(pdb), "observer": record["observer"]}
        record["artifacts"] = bound_files
        record["pdb_binding"] = {"pe_codeview": codeview, "pdb_stream_one": pdb_identity, "guid_and_age_match": True}
        record["working_directory"] = str(cwd)
        record["process_environment_changes"] = {"CARGO_MANIFEST_DIR": str(cwd),
            "observer_private_keys": ["INFINISHELL_EXIT_OBSERVER_BINARY", "INFINISHELL_EXIT_OBSERVER_ROOT", "INFINISHELL_EXIT_OBSERVER_TEST"]}
        for mode in ("normal", "nonzero", "job-delayed", "job-live", "sample"):
            require(source_identity(repo) == record["source"], "source_changed")
            require(file_identity(observer) == bound_files["observer"], "observer_changed")
            result = observer_call(observer, observer, output / ("protocol-" + mode), cwd, "protocol-" + mode)
            record["protocol_tests"].append(result)
            require(result["decision"]["continue"], "native_protocol_" + mode + "_failed")
        started = time.monotonic()
        for index in range(1, MAX_ATTEMPTS + 1):
            require(time.monotonic() - started < EXPERIMENT_BUDGET_SECONDS, "experiment_budget_exhausted")
            require(source_identity(repo) == record["source"], "source_changed")
            require(environment_contract() == record["environment_contract"], "environment_changed")
            before = {key: file_identity(Path(value["path"])) for key, value in bound_files.items()}
            require(before == bound_files, "artifact_identity_changed")
            validate_pdb_match(read_pe_codeview(binary), read_pdb_identity(pdb))
            require(time.monotonic() - started <= EXPERIMENT_BUDGET_SECONDS - 75, "experiment_budget_exhausted")
            result = observer_call(observer, binary, output / f"attempt-{index:03d}", cwd, "observe")
            result["index"] = index
            record["attempts"].append(result)
            after = {key: file_identity(Path(value["path"])) for key, value in bound_files.items()}
            result["artifact_identities_unchanged"] = after == bound_files
            require(after == bound_files and source_identity(repo) == record["source"], "identity_changed_after_observation")
            if not result["decision"]["continue"]:
                raise DiagnosticError(result["decision"]["reason"])
        record["status"] = "not_reproduced_in_40_direct_libtest_observations"
        exit_code = 0
    except BaseException as error:
        record["status"] = "stopped"
        record["error_kind"] = type(error).__name__
        record["stop_reason"] = error.code if isinstance(error, DiagnosticError) else "tool_exception"
        # 不输出异常对象或环境；原始工具/测试输出留在明确的本轮目录。
        with (output / "STOP.json").open("x", encoding="utf-8") as stream:
            json.dump({"reason": record["stop_reason"], "attempts_recorded": len(record["attempts"])}, stream)
    finally:
        record["finished_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        record["attempts_recorded"] = len(record["attempts"])
        record["passed_direct_observations"] = sum(item["decision"]["continue"] for item in record["attempts"])
        with (output / "summary.json").open("x", encoding="utf-8") as stream:
            json.dump(record, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
