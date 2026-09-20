#!/usr/bin/env python3
"""离线验证通知证据和清理边界；不启动 CLI、模型、网络或真实 Win32 子进程。"""

import argparse
import copy
import ctypes
from contextlib import nullcontext
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import uuid

sys.dont_write_bytecode = True
import probe_claude_windows_notifications as probe


REPO = Path(os.environ.get("INFINISHELL_OFFLINE_FIXTURE_REPO", Path(__file__).resolve().parents[2]))


def expected():
    return {"session_id": "602d0e2b-08c5-4873-bc3a-7be085af8d77", "cwd": r"C:\私有 工作区\project"}


def notification():
    return {"v": 1, "agent": "claude", "event": "session_start", "plugin_version": "2.2.0", **expected()}


def osc(value, end=b"\x07"):
    return b"\x1b]777;notify;warp://cli-agent;" + json.dumps(value, ensure_ascii=False).encode() + end


def evidence():
    value = {"schema": 1, "passed": True, "inputs_unchanged": True, "model_inputs_sent": 0,
             "credentials_provided": False, "credential_files_read": False, "full_lifecycle_verified": False,
             "product_installer_exercised": False, "default_first_run_verified": False,
             "path_cases": [], "transport_cases": []}
    for name in probe.CASE_NAMES:
        value["path_cases"].append({"name": name, "passed": True, "phase": "init_only_path",
            "scripts_instrumented": True, "terminal_transport_verified": False, "job_empty": True,
            "forced_cleanup": False, "credential_boundary": True, "native_session_start": True,
            "plugin_root_preserved": True, "injection_marker_absent": True, "assigned_before_resume": True,
            "cleanup_errors": [], "exit_code": 0})
        row = {"name": name, "passed": True, "session_id": str(uuid.uuid4()), "formal_resource_hashes_match": True,
               "conpty": {"console_close_and_output_eof_confirmed": True, "exit_code": 0, "breakaway_removed": True,
                          "original_create_flags": 0x1080400, "actual_create_flags": 0x80400, "create_flags": 0x80400}}
        row["expected"] = {**expected(), "session_id": row["session_id"]}
        row["notification"] = {**notification(), **row["expected"]}
        row["native"] = {"phase": "interactive_conpty", "passed": True, "fixture_completed_onboarding": True,
            "fixture_project_trust": True, "native_child_attached": True, "native_session_start_received": True,
            "formal_resource_hashes_match": True, "job_empty": True, "credential_boundary": True,
            "default_first_run_verified": False, "scripts_instrumented": False, "forced_cleanup": False,
            "assigned_before_resume": True, "cleanup_errors": [], "model_inputs_sent": 0,
            "notification": copy.deepcopy(row["notification"])}
        value["transport_cases"].append(row)
    return value


class ClaudeWindowsNotificationsTests(unittest.TestCase):
    def setUp(self):
        # 纯 schema、文件和清理回归在两平台都运行；遗漏的实体调用立即失败。
        for name in ("subprocess.Popen", "subprocess.run", "socket.create_connection"):
            guard = mock.patch(name, side_effect=AssertionError("离线测试不得启动进程或网络"))
            guard.start()
            self.addCleanup(guard.stop)

    def test_only_literal_osc_with_exact_uuid_and_path_is_accepted(self):
        for ending in (b"\x07", b"\x1b\\"):
            self.assertEqual(probe.transport_match(b"UI text" + osc(notification(), ending), expected()), notification())
        invalid = [json.dumps(notification()).encode(), json.dumps({"terminalSequence": osc(notification()).decode()}).encode(),
                   osc(notification()) * 2, osc(notification()) + osc({"diagnostic_nonce": "test"}),
                   osc([]), osc(None), osc("marker"), b"", b'\x1b]777;notify;warp://cli-agent;bad json\x07']
        for raw in invalid:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                probe.transport_match(raw, expected())

    def test_notification_rejects_false_identity_and_schema(self):
        for key, item in (("v", True), ("v", "1"), ("agent", "codex"), ("event", "stop"),
                          ("session_id", str(uuid.uuid4())), ("cwd", expected()["cwd"].replace("\\", "/")),
                          ("plugin_version", "2.3.0"), ("cwd", [])):
            with self.subTest(key=key, value=item), self.assertRaises(ValueError):
                probe.transport_match(osc({**notification(), key: item}), expected())
        for item in (None, [], {"session_id": [], "cwd": "x"}, {"session_id": "not-uuid", "cwd": "x"}):
            with self.subTest(expected=item), self.assertRaises(ValueError):
                probe.transport_match(osc(notification()), item)

    def test_complete_two_phase_evidence_is_required(self):
        probe.validate_acceptance(evidence())
        for key, item in (("schema", True), ("schema", []), ("model_inputs_sent", False),
                          ("credentials_provided", True), ("credential_files_read", True),
                          ("product_installer_exercised", True), ("default_first_run_verified", True),
                          ("full_lifecycle_verified", True), ("path_cases", {}), ("transport_cases", [])):
            value = evidence()
            value[key] = item
            with self.subTest(key=key), self.assertRaises(ValueError):
                probe.validate_acceptance(value)

    def test_marker_exit_zero_forced_cleanup_or_eof_failure_cannot_pass(self):
        paths = (("path_cases", "plugin_root_preserved", False), ("path_cases", "native_session_start", False),
                 ("path_cases", "assigned_before_resume", False), ("path_cases", "exit_code", False),
                 ("path_cases", "terminal_transport_verified", True), ("path_cases", "cleanup_errors", [{}]),
                 ("transport_cases", "passed", False))
        for group, key, item in paths:
            value = evidence()
            value[group][0][key] = item
            with self.subTest(group=group, key=key), self.assertRaises(ValueError):
                probe.validate_acceptance(value)
        for key, item in (("phase", "init_only_path"), ("fixture_completed_onboarding", False),
                          ("fixture_project_trust", False), ("scripts_instrumented", True),
                          ("forced_cleanup", True), ("job_empty", False), ("native_child_attached", False),
                          ("formal_resource_hashes_match", False), ("model_inputs_sent", False),
                          ("cleanup_errors", [{"error": "synthetic"}])):
            value = evidence()
            value["transport_cases"][0]["native"][key] = item
            with self.subTest(native=key), self.assertRaises(ValueError):
                probe.validate_acceptance(value)
        for key, item in (("exit_code", False), ("exit_code", 1), ("console_close_and_output_eof_confirmed", False),
                          ("breakaway_removed", False), ("original_create_flags", 0x80400), ("actual_create_flags", 0x1080400)):
            value = evidence()
            value["transport_cases"][0]["conpty"][key] = item
            with self.subTest(conpty=key), self.assertRaises(ValueError):
                probe.validate_acceptance(value)

    def test_forged_matching_objects_and_reused_uuid_are_rejected(self):
        value = evidence()
        for row in value["transport_cases"]:
            row["notification"]["agent"] = "codex"
            row["native"]["notification"]["agent"] = "codex"
        with self.assertRaises(ValueError):
            probe.validate_acceptance(value)
        value = evidence()
        value["transport_cases"][1] = {**copy.deepcopy(value["transport_cases"][0]), "name": probe.CASE_NAMES[1]}
        with self.assertRaises(ValueError):
            probe.validate_acceptance(value)
        value = evidence()
        value["transport_cases"][0]["session_id"] = []
        with self.assertRaises(ValueError):
            probe.validate_acceptance(value)

    def test_environment_removes_credentials_startup_scripts_and_fake_version(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {
                "ANTHROPIC_API_KEY": "synthetic-secret", "CLAUDE_CODE_OAUTH_TOKEN": "synthetic-secret",
                "CLAUDE_CODE_VERSION": "fake", "CLAUDE_CODE_ENTRYPOINT": "sdk",
                "BASH_ENV": "unexpected-script", "ENV": "unexpected-script", "PYTHONPATH": "unexpected-code",
                "NODE_OPTIONS": "unexpected-code", "TMUX": "unexpected-server", "ANTHROPIC_BASE_URL": "unexpected-provider"}):
            root = Path(directory).resolve()
            env = probe.case_environment(root, {"PATH": "native-tools", "COMSPEC": "fixed-cmd"}, root / "bin/claude.exe")
            for key in ("ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_VERSION", "CLAUDE_CODE_ENTRYPOINT",
                        "BASH_ENV", "ENV", "PYTHONPATH", "NODE_OPTIONS", "TMUX", "ANTHROPIC_BASE_URL"):
                self.assertNotIn(key, env)
            self.assertEqual(env["PATH"], str(root / "bin") + os.pathsep + "native-tools")
            self.assertEqual(env["WARP_CLI_AGENT_PROTOCOL_VERSION"], "1")
            for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "CLAUDE_CONFIG_DIR"):
                self.assertTrue(Path(env[key]).is_relative_to(root))

    def test_onboarding_is_only_private_config_and_exact_synthetic_project(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            project = root / "自建 工作区"
            project.mkdir()
            probe.case_environment(root, {"PATH": "tools"}, root / "claude.exe")
            result = probe.seed_completed_onboarding(root, project)
            self.assertEqual(set(result), {"hasCompletedOnboarding", "theme", "autoUpdates", "projects"})
            self.assertEqual(result["projects"], {project.as_posix(): {"hasTrustDialogAccepted": True}})
            self.assertEqual(probe.bounded_json(root / "claude/.claude.json"), result)
            with self.assertRaises(FileExistsError):
                probe.seed_completed_onboarding(root, project)

    def test_native_argv_preserves_plugin_and_never_supplies_prompt_or_bypass(self):
        for name in probe.CASE_NAMES:
            plugin = Path(name)
            command = probe.native_command(Path("claude.exe"), plugin, session_id=expected()["session_id"])
            self.assertEqual(command, ["claude.exe", "--session-id", expected()["session_id"], "--setting-sources", "",
                "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--plugin-dir", str(plugin)])
            self.assertIn("--init-only", probe.native_command(Path("claude.exe"), plugin))
        with self.assertRaises(ValueError):
            probe.native_command(Path("claude.exe"), Path("plugin"), session_id="old")

    def test_formal_tree_is_real_resource_combination_without_instrumentation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            plugin = root / probe.CASE_NAMES[1]
            hashes = probe.prepare_plugin(REPO, plugin, root)
            probe.verify_plugin(plugin, hashes)
            self.assertEqual(len(hashes), 18)
            originals = json.loads((REPO / "specs/cli-agent-parity/fixtures/claude-warp-compatible-original-trees.json").read_text(encoding="utf-8"))["2.2.0"]
            self.assertEqual((plugin / "scripts/on-session-start.sh").read_text(encoding="utf-8"), originals["scripts/on-session-start.sh"])
            bundled = REPO / "app/assets/bundled/cli-agent-plugins/claude"
            for path in ("hooks/hooks.json", "scripts/warp-notify.sh"):
                self.assertEqual((plugin / path).read_bytes(), (bundled / path).read_bytes())
            hooks = json.loads((plugin / "hooks/hooks.json").read_text(encoding="utf-8"))["hooks"]
            self.assertEqual(len(hooks), 7)
            (plugin / "unrecognized.tmp").write_bytes(b"interruption")
            with self.assertRaises(ValueError):
                probe.verify_plugin(plugin, hashes)

    def test_private_config_rejects_outside_aliases_and_bad_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / ".probe-owned").write_bytes(probe.MARKER)
            path = root / "worker.config.json"
            config = {"schema": 1, "private_root": str(root), "worker_report": str(root / "worker.json")}
            probe.write_json(path, config, exclusive=True)
            self.assertEqual(probe.load_configuration(path), config)
            for key, item in (("schema", True), ("private_root", []), ("worker_report", str(root.parent / "outside.json")),
                              ("raw_output", str(root / ".." / "outside.bin"))):
                probe.write_json(path, {**config, key: item})
                with self.subTest(key=key), self.assertRaises(ValueError):
                    probe.load_configuration(path)
            probe.write_json(path, config)
            (root / ".probe-owned").write_bytes(b"unknown")
            with self.assertRaises(ValueError):
                probe.load_configuration(path)

    def test_private_path_rejects_reparse_and_symbolic_nodes_before_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            child = root / "child"
            child.mkdir()
            with mock.patch.object(Path, "is_symlink", return_value=True), self.assertRaises(ValueError):
                probe.private_path(root, str(child / "output.json"))

    def test_atomic_receipt_update_and_exclusive_reservation(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            probe.write_json(path, {"passed": False}, exclusive=True)
            with self.assertRaises(FileExistsError):
                probe.write_json(path, {"passed": True}, exclusive=True)
            with mock.patch.object(probe.os, "replace", side_effect=OSError("synthetic failure")), self.assertRaises(OSError):
                probe.write_json(path, {"passed": True})
            self.assertEqual(probe.bounded_json(path), {"passed": False})
            self.assertEqual(list(path.parent.iterdir()), [path])

    def test_cleanup_always_closes_job_and_marks_force_without_claiming_success(self):
        process = mock.Mock()
        process.poll.return_value = None
        job = mock.Mock()
        job.active.return_value = 1
        job.wait_empty.return_value = 0
        record = {"forced_cleanup": False}
        probe.cleanup_process(process, job, record)
        self.assertTrue(record["forced_cleanup"])
        self.assertTrue(record["job_empty"])
        job.terminate.assert_called_once()
        job.close.assert_called_once()
        for operation in ("active", "wait_empty", "close"):
            job = mock.Mock()
            job.active.return_value = 0
            job.wait_empty.return_value = 0
            getattr(job, operation).side_effect = OSError("synthetic secret must not escape")
            with self.subTest(operation=operation), self.assertRaises(ValueError):
                probe.cleanup_process(process, job, {})
            job.close.assert_called_once()

    def test_failures_do_not_archive_native_text_or_foreign_paths(self):
        marker = "synthetic-private-data"
        self.assertNotIn(marker, json.dumps(probe.failure(OSError(marker))))
        record = {"startup_failure": {"message": marker, "filename": marker}, "handle_close_failures": [{"message": marker}]}
        probe.sanitize_trace(record)
        self.assertNotIn(marker, json.dumps(record))
        self.assertEqual(probe.failure(probe.ProbeFailure("固定边界失败"))["reason"], "固定边界失败")

    def test_dependency_selection_rejects_wsl_and_non_native_jq_without_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            bash, jq = root / "bash.exe", root / "jq.exe"
            bash.write_bytes(b"synthetic binary")
            jq.write_bytes(b"synthetic binary")
            with self.assertRaises(ValueError):
                probe.select_dependencies(bash, jq)
            (root / "msys-2.0.dll").write_bytes(b"synthetic marker")
            self.assertEqual(probe.select_dependencies(bash, jq), (bash, jq))
            with self.assertRaises(ValueError):
                probe.select_dependencies(bash, root / "jq.cmd")

    def test_controller_timeout_recovers_receipts_then_removes_only_owned_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            private = root / "owned"
            private.mkdir()
            outside = root / "outside.txt"
            outside.write_bytes(b"do not change")
            args = argparse.Namespace(output=root / "report.json", executable=root / "claude.exe",
                                      bash_executable=root / "bash.exe", jq_executable=root / "jq.exe")
            process, job = mock.Mock(), mock.Mock()
            process.wait.side_effect = [subprocess.TimeoutExpired("synthetic-worker", 420), 0]
            process.poll.return_value = None
            job.active.return_value = 1
            job.wait_empty.return_value = 0
            def launch(*unused, **unused_keywords):
                probe.write_json(private / "worker.json", {"schema": 1, "passed": False, "path_cases": [{"passed": False}]})
                probe.write_json(private / "case-0.driver.json", {"passed": False, "native_child_attached": True})
                return process
            with mock.patch.object(probe, "require_native_host"), mock.patch.object(probe, "verify_binary", return_value={"sha256": "fixed"}), \
                 mock.patch.object(probe, "select_dependencies", return_value=(args.bash_executable, args.jq_executable)), \
                 mock.patch.object(probe.tempfile, "mkdtemp", return_value=str(private)), \
                 mock.patch.object(probe, "identity", return_value="a" * 40), \
                 mock.patch.dict(sys.modules, {"_winapi": mock.Mock()}), \
                 mock.patch.object(probe, "WindowsProbeJob", return_value=job), \
                 mock.patch.object(probe, "suspended_creation", return_value=nullcontext()), \
                 mock.patch.object(probe.subprocess, "Popen", side_effect=launch), mock.patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(probe.public_run(args), 1)
            report = probe.bounded_json(args.output)
            self.assertFalse(report["passed"])
            self.assertTrue(report["forced_cleanup"])
            self.assertTrue(report["private_directory_removed"])
            self.assertEqual(set(report["recovered_receipts"]), {"worker.json", "case-0.driver.json"})
            self.assertFalse(private.exists())
            self.assertEqual(outside.read_bytes(), b"do not change")
            job.terminate.assert_called_once()
            job.close.assert_called_once()

    def test_unsupported_platform_records_failure_without_process_or_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(output=Path(directory).resolve() / "report.json", executable=None,
                                      bash_executable=None, jq_executable=None)
            with mock.patch.object(probe, "require_native_host", side_effect=probe.ProbeFailure("需要原生 Windows x64 CPython")), \
                 mock.patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(probe.public_run(args), 1)
                with self.assertRaises(FileExistsError):
                    probe.public_run(args)
            self.assertFalse(probe.bounded_json(args.output)["passed"])

    def test_interactive_driver_inherits_conpty_and_sends_only_exit_keys(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / ".probe-owned").write_bytes(probe.MARKER)
            case_root = root / "case"
            project, plugin = case_root / "project", case_root / "plugin"
            project.mkdir(parents=True)
            plugin.mkdir()
            session_id = str(uuid.uuid4())
            raw = root / "pty.bin"
            raw.write_bytes(osc({**notification(), "session_id": session_id, "cwd": str(project)}))
            config = {"schema": 1, "private_root": str(root), "case_root": str(case_root), "project": str(project),
                      "plugin": str(plugin), "executable": str(root / "claude.exe"), "dependencies": {"PATH": "tools"},
                      "session_id": session_id, "raw_output": str(raw), "driver_report": str(root / "driver.json"),
                      "plugin_hashes": {}}
            config_path = root / "driver.config.json"
            probe.write_json(config_path, config, exclusive=True)
            process, job, api = mock.Mock(), mock.Mock(), mock.Mock()
            process.pid, process.returncode = 42, 0
            process.poll.side_effect = [None, 0, 0]
            job.active.return_value = 0
            job.wait_empty.return_value = 0
            api.console_members.return_value = [{"pid": 42}]
            def attached(*arguments):
                arguments[-1]["assigned_before_resume"] = True
                return nullcontext()
            with mock.patch.dict(sys.modules, {"_winapi": mock.Mock()}), \
                 mock.patch.object(probe, "WindowsProbeJob", return_value=job), \
                 mock.patch.object(probe, "verify_binary"), mock.patch.object(probe, "WinApi", return_value=api), \
                 mock.patch.object(probe, "suspended_creation", side_effect=attached), \
                 mock.patch.object(probe.subprocess, "Popen", return_value=process) as launch, \
                 mock.patch.object(probe, "send_exit_key") as exit_key, mock.patch.object(probe.time, "sleep"):
                self.assertEqual(probe.run_driver(config_path), 0)
            self.assertEqual(set(launch.call_args.kwargs), {"env", "cwd"})
            exit_key.assert_called_once_with(api)
            result = probe.bounded_json(root / "driver.json")
            self.assertTrue(result["passed"])
            self.assertTrue(result["native_session_start_received"])
            self.assertEqual(result["model_inputs_sent"], 0)
            self.assertFalse(result["scripts_instrumented"])

    def test_worker_early_failure_still_writes_failed_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / ".probe-owned").write_bytes(probe.MARKER)
            config = {"schema": 1, "repo": str(REPO), "private_root": str(root), "executable": str(root / "claude.exe"),
                      "worker_report": str(root / "worker.json")}
            config_path = root / "worker.config.json"
            probe.write_json(config_path, config, exclusive=True)
            with mock.patch.object(probe, "verify_binary", side_effect=ValueError("synthetic native failure")):
                self.assertEqual(probe.run_worker(config_path), 1)
            report = probe.bounded_json(root / "worker.json")
            self.assertFalse(report["passed"])
            self.assertEqual(report["path_cases"], [])
            self.assertNotIn("synthetic native failure", json.dumps(report))

    def test_worker_preserves_completed_path_receipt_on_later_exception(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / ".probe-owned").write_bytes(probe.MARKER)
            config = {"schema": 1, "repo": str(REPO), "private_root": str(root), "executable": str(root / "claude.exe"),
                      "worker_report": str(root / "worker.json"), "bash": str(root / "bash.exe"), "jq": str(root / "jq.exe")}
            config_path = root / "worker.config.json"
            probe.write_json(config_path, config, exclusive=True)
            first = {"passed": False, "name": probe.CASE_NAMES[0], "phase": "init_only_path"}
            with mock.patch.object(probe, "verify_binary", return_value={}), mock.patch.object(probe, "verify_version", return_value="fixed"), \
                 mock.patch.object(probe, "windows_environment", return_value={"PATH": "tools"}), \
                 mock.patch.object(probe, "digest", return_value="f" * 64), mock.patch.object(probe, "source_hashes", return_value={}), \
                 mock.patch.object(probe.shutil, "copyfile"), \
                 mock.patch.object(probe, "run_path_case", side_effect=[first, OSError("synthetic creation failure")]):
                self.assertEqual(probe.run_worker(config_path), 1)
            report = probe.bounded_json(root / "worker.json")
            self.assertEqual(report["path_cases"], [first])
            self.assertFalse(report["passed"])

    def test_only_exact_conpty_driver_loses_breakaway_flag(self):
        command = ["python.exe", "-B", "private probe.py", "--driver-config", "owned.json"]
        cwd = Path("owned")
        arguments = [command[0], ctypes.create_unicode_buffer(subprocess.list2cmdline(command)), None, None,
                     False, 0x1080400, object(), str(cwd), object(), object()]
        native, trace = mock.Mock(return_value=1), {}
        create = probe.contained_driver_creation(native, command, cwd, trace)
        self.assertEqual(create(*arguments), 1)
        forwarded = native.call_args.args
        self.assertEqual(forwarded[:5], tuple(arguments[:5]))
        self.assertEqual(forwarded[5], 0x80400)
        self.assertEqual(forwarded[6:], tuple(arguments[6:]))
        self.assertEqual(trace, {"original_create_flags": 0x1080400, "actual_create_flags": 0x80400, "breakaway_removed": True})
        with self.assertRaises(ValueError):
            create(*arguments)
        native.assert_called_once()
        for index, value in ((0, "other.exe"), (1, ctypes.create_unicode_buffer("wrong driver")),
                             (4, True), (5, 0x80400), (5, 0x1080404), (7, "outside")):
            invalid = arguments.copy()
            invalid[index] = value
            native = mock.Mock()
            with self.subTest(index=index, value=value), self.assertRaises(ValueError):
                probe.contained_driver_creation(native, command, cwd, {})(*invalid)
            native.assert_not_called()

    def test_contained_host_records_actual_flags_and_restores_factory(self):
        command = ["python.exe", "--driver-config", "private.json"]
        cwd, trace = Path("owned"), {}
        native = mock.Mock(return_value=1)
        class FakeApi:
            def __init__(self):
                self.kernel = mock.Mock(CreateProcessW=native)
        def host(*arguments):
            api = probe.conpty_host.WinApi()
            api.kernel.CreateProcessW(command[0], ctypes.create_unicode_buffer(subprocess.list2cmdline(command)),
                                      None, None, False, 0x1080400, None, str(cwd), None, None)
            return {"create_flags": 0x1080400, "exit_code": 0}
        with mock.patch.object(probe.conpty_host, "WinApi", FakeApi), mock.patch.object(probe, "run_conpty", side_effect=host):
            report = probe.run_contained_conpty(Path("conpty.dll"), command, {}, cwd, Path("output.bin"), 90, trace)
            self.assertIs(probe.conpty_host.WinApi, FakeApi)
        self.assertEqual(report["create_flags"], 0x80400)
        self.assertEqual(report["original_create_flags"], 0x1080400)
        self.assertEqual(report["actual_create_flags"], 0x80400)
        self.assertTrue(report["breakaway_removed"])
        self.assertEqual(native.call_args.args[5], 0x80400)

    def test_conpty_factory_is_restored_after_host_exception(self):
        original = probe.conpty_host.WinApi
        with mock.patch.object(probe, "run_conpty", side_effect=OSError("synthetic host failure")), self.assertRaises(OSError):
            probe.run_contained_conpty(Path("conpty.dll"), ["python.exe"], {}, Path("owned"), Path("output.bin"), 90, {})
        self.assertIs(probe.conpty_host.WinApi, original)

    def test_fixed_binary_mismatch_fails_without_invocation(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "claude.exe"
            path.write_bytes(b"not fixed Claude")
            with self.assertRaises(ValueError):
                probe.verify_binary(path, "win32-x64")


if __name__ == "__main__":
    unittest.main()
