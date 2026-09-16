#!/usr/bin/env python3
"""仅测试真实工具探针的身份、审批与证据拒绝路径；不派生 CLI 或发送信号。"""

import copy
import errno
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import uuid
from types import SimpleNamespace
from unittest.mock import Mock

import probe_codex_supervisor_tool_tree as probe


def identity(pid, cid=99, version=2):
    return {"pid": pid, "pid_version": version, "unique_id": pid * 10, "resource_cid": cid}


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.generation = str(uuid.uuid4())
        self.state = self.root / self.generation
        self.state.mkdir(mode=0o700)
        self.codex, self.supervisor, self.python = (self.root / name for name in ("codex", "supervisor", "python"))
        self.project = self.root / "project"
        self.boot = str(uuid.uuid4())
        self.manifest = {"version": 1, "generation": self.generation, "launch_allowed": True,
                         "executable": str(self.codex), "cwd": str(self.project),
                         "arguments": [{"Unix": list(os.fsencode(item))} for item in probe.CODEX_ARGUMENTS]}
        self.save("manifest.json", self.manifest)
        self.claim = {"version": 1, "generation": self.generation,
                      "manifest_sha256": probe.private_record(self.state / "manifest.json")[1],
                      "label": "dev.infinishell.cli-agent." + self.generation,
                      "boot_session": self.boot, "wrapper": identity(30)}
        self.save("macos-coalition.json", self.claim)
        self.native = {"generation": self.generation, "identity": identity(40, version=1),
                       "claim_sha256": probe.private_record(self.state / "macos-coalition.json")[1]}
        self.save("macos-native.json", self.native)
        self.initial = self.load()
        self.receipt = {"version": 1, "generation": self.generation, "containment": probe.CONTAINMENT,
                        "cleanup_confirmed": True, "exit_code": None, "exit_reason": "stdio_closed",
                        "manifest_sha256": self.initial["sha256"]["manifest.json"]}
        self.proof = {"version": 1, "generation": self.generation, "job_removed": True,
                      "resource_cid_destroyed": True, "native_wait_status": 9, "execution_failed": False,
                      "claim_sha256": self.initial["sha256"]["macos-coalition.json"],
                      "native_sha256": self.initial["sha256"]["macos-native.json"]}
        self.save("exit.json", self.receipt)
        self.save("macos-cleanup.json", self.proof)

    def save(self, name, value):
        probe.write(self.state / name, value)

    def load(self):
        return probe.launch_records(self.state, self.generation, self.codex, self.project)

    def verify(self):
        return probe.verify_proof(self.state, self.initial, self.generation)

    def test_complete_bound_production_proof_is_accepted(self):
        self.assertEqual(self.verify()["proof"], self.proof)

    def test_cleanup_proof_does_not_require_a_successful_cli_exit(self):
        self.proof.update(execution_failed=True, native_wait_status=None)
        self.save("macos-cleanup.json", self.proof)
        self.assertTrue(self.verify()["proof"]["execution_failed"])
        self.receipt["exit_code"] = 0
        self.save("exit.json", self.receipt)
        with self.assertRaises(RuntimeError):
            self.verify()

    def test_foreign_native_cid_or_generation_rejected_before_signal(self):
        for key, value in [("generation", str(uuid.uuid4())), ("identity", identity(40, cid=101))]:
            with self.subTest(key=key):
                broken = {**self.native, key: value}
                self.save("macos-native.json", broken)
                with self.assertRaises(RuntimeError):
                    self.load()

    def test_wrong_manifest_image_or_argv_is_rejected(self):
        for key, value in [("executable", "/tmp/other-cli"), ("arguments", [])]:
            with self.subTest(key=key):
                self.save("manifest.json", {**self.manifest, key: value})
                with self.assertRaises(RuntimeError):
                    self.load()

    def test_initial_records_cannot_be_rewritten_even_as_equivalent_json(self):
        path = self.state / "macos-native.json"
        path.write_bytes(path.read_bytes() + b"\n")
        with self.assertRaises(RuntimeError):
            self.verify()

    def test_partial_or_foreign_proof_and_nonterminal_wait_are_rejected(self):
        for key, value in [("job_removed", False), ("resource_cid_destroyed", False),
                           ("generation", str(uuid.uuid4())), ("claim_sha256", "0" * 64),
                           ("native_sha256", "0" * 64), ("native_wait_status", 0x137f)]:
            with self.subTest(key=key):
                self.save("macos-cleanup.json", {**self.proof, key: value})
                with self.assertRaises(RuntimeError):
                    self.verify()

    def test_json_booleans_cannot_impersonate_native_numeric_fields(self):
        self.save("macos-cleanup.json", {**self.proof, "version": True})
        with self.assertRaises(RuntimeError): self.verify()
        self.save("macos-cleanup.json", {**self.proof, "native_wait_status": False})
        with self.assertRaises(RuntimeError): self.verify()
        self.save("macos-cleanup.json", {**self.proof, "native_wait_status": 0})
        self.save("exit.json", {**self.receipt, "exit_code": False})
        with self.assertRaises(RuntimeError): self.verify()

    def test_old_process_group_receipt_cannot_upgrade_to_coalition(self):
        self.receipt["containment"] = "unix_process_group"
        self.save("exit.json", self.receipt)
        with self.assertRaises(RuntimeError):
            self.verify()
        report = self.complete_report()
        report["receipt"]["containment"] = "unix_process_group"
        self.assertFalse(probe.classify_cleanup(report)["passed"])

    def test_linked_public_missing_and_oversized_records_rejected(self):
        path = self.state / "macos-cleanup.json"
        for mode in ["missing", "public", "oversized", "symlink", "hardlink"]:
            with self.subTest(mode=mode):
                path.unlink(missing_ok=True)
                self.save("macos-cleanup.json", self.proof)
                if mode == "missing": path.unlink()
                elif mode == "public": path.chmod(0o644)
                elif mode == "oversized": path.write_bytes(b" " * 65537)
                else:
                    other = self.state / (mode + ".json")
                    probe.write(other, self.proof)
                    path.unlink()
                    if mode == "symlink": path.symlink_to(other)
                    else: os.link(other, path)
                with self.assertRaises((RuntimeError, OSError)):
                    self.verify()

    def complete_report(self):
        return {"receipt": copy.deepcopy(self.receipt), "generation": self.generation,
                "manifest_digest_matches": True, "tool_identity": {"pid": 50, "sleep_pid": 60},
                "required_note_exit_pids": [30, 40, 50, 60], "note_exit_pids": [30, 40, 50, 60],
                "production_proof_verified": True, "same_boot_cid_esrch": True,
                "exact_approval_verified": True, "crash_target_verified": True,
                "heartbeat_continued_after_receipt": False}

    def test_stable_heartbeat_or_proof_alone_never_proves_cleanup(self):
        self.assertTrue(probe.classify_cleanup(self.complete_report())["passed"])
        for key, value in [("production_proof_verified", False), ("same_boot_cid_esrch", False),
                           ("note_exit_pids", [30, 40, 50]), ("required_note_exit_pids", []),
                           ("heartbeat_continued_after_receipt", True), ("exact_approval_verified", False),
                           ("crash_target_verified", False)]:
            with self.subTest(key=key):
                self.assertFalse(probe.classify_cleanup({**self.complete_report(), key: value})["passed"])

    def live_tree(self):
        def row(pid, ppid, image, argv, cid=99):
            return {**identity(pid, cid), "ppid": ppid, "parent_unique_id": ppid * 10,
                    "image": str(image), "argv": argv, "pgid": pid, "sid": pid}
        manifest = str(self.state / "manifest.json")
        return {
            10: row(10, 1, self.python, [str(self.python), "--host"], 11),
            20: row(20, 10, self.supervisor, [str(self.supervisor), "cli-agent-supervisor", manifest], 11),
            30: row(30, 1, self.supervisor, [str(self.supervisor), "cli-agent-supervisor", manifest, "--execute"]),
            40: row(40, 30, self.codex, [str(self.codex), *probe.CODEX_ARGUMENTS]),
            50: row(50, 40, self.python, [str(self.python), "heartbeat.py"]),
            60: row(60, 50, "/bin/sleep", ["/bin/sleep", "60"]),
        }

    def bind(self, rows):
        class FakeAPI:
            def boot(_self): return self.boot
            def inspect(_self, pid): return rows[pid].copy()
            def identity(_self, pid): return rows[pid].copy()
        return probe.bind_tree(FakeAPI(), self.initial, {"pid": 20}, rows[10], {"pid": 50, "sleep_pid": 60},
                               self.codex, self.supervisor, self.state, self.python)

    def test_wrapper_native_chain_accepts_exec_version_change_not_direct_supervisor_parent(self):
        bound = self.bind(self.live_tree())
        self.assertEqual(bound["codex"]["ppid"], 30)
        self.assertNotEqual(bound["codex"]["ppid"], bound["supervisor"]["pid"])

    def test_wrong_image_argv_parent_unique_and_cid_each_reject(self):
        for pid, key, value in [(40, "image", "/bin/sleep"), (40, "argv", [str(self.codex), "--version"]),
                                (40, "ppid", 20), (40, "unique_id", 1234), (30, "pid_version", 99),
                                (50, "resource_cid", 12), (60, "parent_unique_id", 777)]:
            with self.subTest(pid=pid, key=key):
                rows = self.live_tree(); rows[pid][key] = value
                with self.assertRaises(RuntimeError):
                    self.bind(rows)


class ParserTests(unittest.TestCase):
    def test_existing_negative_process_group_evidence_remains_negative(self):
        fixtures = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures"
        for name in ["codex-0.147-supervised-cli-crash-macos.json", "codex-0.147-supervised-cli-crash-macos-recovery-gated.json"]:
            with self.subTest(name=name):
                report = json.loads((fixtures / name).read_text())
                self.assertFalse(probe.classify_cleanup(report)["passed"])

    def test_argv_preserves_unicode_empty_arguments_and_ignores_environment(self):
        data = struct.pack("=i", 3) + b"/tmp/codex\0\0\0" + "/tmp/codex\0中文 English\0\0SECRET=do-not-save\0".encode()
        self.assertEqual(probe.parse_argv(data), ["/tmp/codex", "中文 English", ""])

    def test_partial_argv_rejected(self):
        for data in [b"", struct.pack("=i", -1) + b"\0", struct.pack("=i", 2) + b"/bin/x\0/bin/x\0"]:
            with self.assertRaises(RuntimeError): probe.parse_argv(data)

    def test_exact_approval_preserves_quoted_path_but_rejects_extra_command(self):
        expected = ["/tmp/中文 Python", "heartbeat.py"]
        project = Path("/tmp/项目")
        command = probe.shlex.join(expected)
        details = {"command": command, "cwd": str(project), "commandActions": [{"command": command}]}
        self.assertTrue(probe.safe_approval(details, expected, project))
        for changed in [{"command": command + "; touch outside"}, {"cwd": "/tmp/other"},
                        {"commandActions": []}, {"command": None}, {"command": "'"}]:
            self.assertFalse(probe.safe_approval({**details, **changed}, expected, project))

    def test_only_exec_version_may_refresh_before_signal(self):
        saved = identity(100)
        probe.match_identity({**saved, "pid_version": 3}, saved, allow_exec=True)
        for key in ["pid", "unique_id", "resource_cid", "pid_version"]:
            with self.assertRaises(RuntimeError):
                probe.match_identity({**saved, key: saved[key] + 1}, saved)
        with self.assertRaises(RuntimeError):
            probe.match_identity({**saved, "unique_id": 9999}, saved, allow_exec=True)

    def test_signal_rechecks_identity_and_boot_before_native_call(self):
        api = probe.DarwinProcesses.__new__(probe.DarwinProcesses)
        api.boot = lambda: "current"
        api.identity = Mock(return_value=identity(10))
        native = Mock(return_value=0)
        api.lib = SimpleNamespace(proc_signal_with_audittoken=native)
        for saved, boot in [(identity(10), "old"), (identity(10, version=3), "current"),
                            ({**identity(10), "unique_id": 999}, "current")]:
            with self.assertRaises(RuntimeError): api.kill(saved, boot)
        native.assert_not_called()
        api.kill(identity(10), "current")
        token = native.call_args.args[0]._obj
        self.assertEqual((token[5], token[7]), (10, 2))

    def test_only_exact_esrch_in_same_boot_confirms_domain_destruction(self):
        api = probe.DarwinProcesses.__new__(probe.DarwinProcesses)
        api.boot = lambda: "current"
        native = Mock(return_value=0)
        api.system = SimpleNamespace(coalition_info_resource_usage=native)
        for cid, boot in [(0, "current"), (99, "old")]:
            with self.assertRaises(RuntimeError): api.destroyed(cid, boot)
        native.assert_not_called()
        self.assertFalse(api.destroyed(99, "current"))
        def result(error):
            def invoke(*_args):
                probe.ctypes.set_errno(error)
                return -1
            return invoke
        native.side_effect = result(errno.EPERM)
        with self.assertRaises(RuntimeError): api.destroyed(99, "current")
        native.side_effect = result(errno.ESRCH)
        self.assertTrue(api.destroyed(99, "current"))

    @unittest.skipUnless(hasattr(probe.select, "kqueue"), "解析测试不模拟目标平台缺失的 kqueue API")
    def test_registration_race_rejects_new_exec_version_and_closes_observer(self):
        queue = Mock()
        queue.control.return_value = []
        api = SimpleNamespace(identity=Mock(side_effect=[identity(10), identity(10, version=3)]))
        with patch.object(probe.select, "kqueue", return_value=queue):
            with self.assertRaises(RuntimeError): probe.ExitWatch(api, [identity(10)])
        queue.close.assert_called_once()


if __name__ == "__main__":
    unittest.main()
