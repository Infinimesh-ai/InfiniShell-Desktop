"""环境预检的只读边界与收据回归；不创建真实 namespace 或读取 Windows 注册表。"""

import ctypes
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch


SOURCE = Path(__file__).with_name("probe_package_source_environment.py")
SPEC = importlib.util.spec_from_file_location("package_environment", SOURCE)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


def identity():
    return {
        "uid": 1000, "euid": 1000, "gid": 1000, "egid": 1000,
        "uid_map": {"state": "read", "ranges": [[0, 0, 4294967295]]},
        "gid_map": {"state": "read", "ranges": [[0, 0, 4294967295]]},
        "namespaces": {name: {"state": "read", "device": 4, "inode": inode}
                       for name, inode in (("user", 10), ("mnt", 11))},
        "standard_path_identity": {
            "/": {"state": "present", "directory": True, "symlink": False,
                  "uid": 0, "gid": 0, "mode": 493},
            PROBE.BREW_PREFIX: {"state": "absent"},
        },
    }


def child_identity():
    result = identity()
    result.update(uid=0, euid=0, gid=0, egid=0)
    result["uid_map"]["ranges"] = [[0, 1000, 1]]
    result["gid_map"]["ranges"] = [[0, 1000, 1]]
    result["standard_path_identity"]["/"]["uid"] = 65534
    result["standard_path_identity"]["/"]["gid"] = 65534
    for value in result["namespaces"].values():
        value["inode"] += 100
    return result


class PackageSourceEnvironmentTests(unittest.TestCase):
    def run_namespace(self, raw, returncode=0):
        process = MagicMock(returncode=returncode, pid=1234)
        process.communicate.return_value = (raw, None)
        with patch.object(PROBE.os.path, "isfile", return_value=True), \
             patch.object(PROBE.os, "access", return_value=True), \
             patch.object(PROBE.subprocess, "Popen", return_value=process) as spawn:
            result = PROBE.probe_namespaces(identity())
        return result, spawn

    def test_rootless_success_records_uid_difference_without_claiming_transactions(self):
        result, spawn = self.run_namespace(json.dumps(child_identity()).encode())
        self.assertEqual(result["state"], "created")
        self.assertEqual(result["child"]["standard_path_identity"]["/"]["uid"], 65534)
        arguments, options = spawn.call_args
        command = arguments[0]
        self.assertEqual(command[command.index("--propagation") + 1], "unchanged")
        self.assertIn("--namespace-child", command)
        self.assertIn("-I", command)
        self.assertIn("-B", command)
        self.assertTrue(options["start_new_session"])
        self.assertEqual(options["stderr"], subprocess.DEVNULL)
        self.assertEqual(set(options["env"]), {"PATH", "LANG", "LC_ALL"})
        self.assertFalse(any(item in command for item in ("mount", "mkdir", "sh", "bash")))
        with patch.object(PROBE.platform, "system", return_value="Linux"), \
             patch.object(PROBE, "linux_identity", return_value=identity()), \
             patch.object(PROBE, "probe_namespaces", return_value=result), \
             patch.object(PROBE, "landlock_abi", return_value={"state": "available", "abi": 6}):
            receipt = PROBE.probe()
        self.assertFalse(receipt["transaction_execution_verified"])
        self.assertFalse(receipt["filesystem_mounts_performed"])
        self.assertEqual(receipt["linux"]["production_uid_contract"], "requires_validation")
        self.assertEqual(receipt["linux"]["bind_mount_capability"], "not_tested")

    def test_namespace_command_success_without_new_namespaces_is_not_accepted(self):
        result, _ = self.run_namespace(json.dumps(identity()).encode())
        self.assertEqual(result["state"], "identity_not_distinct")

    def test_failed_unshare_does_not_emit_its_output(self):
        result, _ = self.run_namespace(b"unexpected private output", returncode=1)
        self.assertEqual(result, {"state": "unavailable", "exit_code": 1})

    def test_child_receipt_rejects_extra_fields_and_free_text(self):
        extra = child_identity()
        extra["credential"] = "must_not_escape"
        text = child_identity()
        text["standard_path_identity"]["/"]["uid"] = "must_not_escape"
        for child in (extra, text):
            with self.subTest(child=type(child)):
                result, _ = self.run_namespace(json.dumps(child).encode())
                self.assertEqual(result, {"state": "invalid_child_receipt"})

    def test_namespace_timeout_cleans_only_its_own_process_group(self):
        process = MagicMock(pid=1234)
        process.communicate.side_effect = [subprocess.TimeoutExpired("unshare", 10), (b"", None)]
        with patch.object(PROBE.os.path, "isfile", return_value=True), \
             patch.object(PROBE.os, "access", return_value=True), \
             patch.object(PROBE.subprocess, "Popen", return_value=process), \
             patch.object(PROBE.signal, "SIGKILL", 9, create=True), \
             patch.object(PROBE.os, "killpg", create=True) as kill:
            self.assertEqual(PROBE.probe_namespaces(identity()), {"state": "timeout"})
        kill.assert_called_once_with(1234, 9)

    def test_landlock_only_queries_the_abi(self):
        syscall = MagicMock(return_value=3)
        with patch.object(PROBE.platform, "machine", return_value="x86_64"), \
             patch.object(PROBE.ctypes, "CDLL", return_value=SimpleNamespace(syscall=syscall)):
            self.assertEqual(PROBE.landlock_abi(), {"state": "available", "abi": 3,
                                                  "abi3_available": True})
        self.assertEqual([argument.value for argument in syscall.call_args.args], [444, None, 0, 1])

    def test_landlock_failure_keeps_numeric_errno_only(self):
        def unavailable(*arguments):
            ctypes.set_errno(38)
            return -1

        syscall = MagicMock(side_effect=unavailable)
        with patch.object(PROBE.platform, "machine", return_value="x86_64"), \
             patch.object(PROBE.ctypes, "CDLL", return_value=SimpleNamespace(syscall=syscall)):
            self.assertEqual(PROBE.landlock_abi(), {"state": "unavailable", "error_code": 38})

    def test_windows_only_opens_hkcu_and_does_not_execute_winget(self):
        registry = SimpleNamespace(HKEY_CURRENT_USER=1, KEY_READ=2, OpenKey=MagicMock())
        shell = SimpleNamespace(IsUserAnAdmin=MagicMock(return_value=0))
        with patch.dict(sys.modules, {"winreg": registry}), \
             patch.object(PROBE.ctypes, "WinDLL", return_value=shell, create=True), \
             patch.object(PROBE.shutil, "which", return_value=r"C:\private-user\winget.exe"), \
             patch.object(PROBE, "windows_profile", return_value={"state": "profile_readable"}), \
             patch.object(PROBE.subprocess, "Popen") as spawn:
            result = PROBE.probe_windows()
        registry.OpenKey.assert_called_once_with(1, "", 0, 2)
        spawn.assert_not_called()
        self.assertEqual(result["winget"], {"command_found": True, "execution_checked": False})
        self.assertFalse(result["effective_admin_membership"])
        self.assertNotIn("private-user", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
