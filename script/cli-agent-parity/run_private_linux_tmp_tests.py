"""Linux 私有 TMPDIR 包装器的离线合同；不启动门禁、模型或平台子进程。"""

import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import run_private_linux_tmp as runner


def proc(pid, parent, start=10, state="S"):
    return {"pid": pid, "parent_pid": parent, "start_time_ticks": start, "state": state}


class PrivateLinuxTmpTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / "t-fixture"
        self.root.mkdir(mode=0o700)
        self.descriptor = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        self.addCleanup(os.close, self.descriptor)
        self.identity = runner.identity(os.fstat(self.descriptor))

    def test_descendant_tracking_covers_adopted_and_nested_children_without_process_groups(self):
        table = {1: proc(1, 0), 10: proc(10, 1), 11: proc(11, 10),
                 12: proc(12, 11), 13: proc(13, 10), 14: proc(14, 1)}
        self.assertEqual({value["pid"] for value in runner.descendants(table, 10)}, {11, 12, 13})
        self.assertNotEqual(runner.process_key(proc(11, 10, 20)),
                            runner.process_key(proc(11, 10, 21)))

    def test_process_stat_handles_spaces_and_parentheses_in_command(self):
        fields = ["S", "9"] + ["0"] * 17 + ["1234", "0"]
        with patch.object(Path, "read_text", return_value="17 (a ) b) " + " ".join(fields)):
            self.assertEqual(runner.process(17), proc(17, 9, 1234))

    def test_root_requires_real_account_home_and_checks_budget_before_creating(self):
        home = Path("/" + "x" * 40)
        with (patch.object(runner.pwd, "getpwuid", return_value=SimpleNamespace(pw_dir=str(home))),
              patch.object(runner, "check_ancestors"),
              patch.object(Path, "lstat", return_value=self.root.lstat()),
              patch.object(runner.tempfile, "mkdtemp") as create):
            with self.assertRaisesRegex(ValueError, "45"):
                runner.create_root(lambda path: None)
            create.assert_not_called()

    def test_unsafe_ancestor_and_symlink_are_rejected(self):
        info = SimpleNamespace(st_mode=0o40777, st_uid=os.geteuid())
        with patch.object(Path, "lstat", return_value=info):
            with self.assertRaises(ValueError):
                runner.check_ancestors(self.root, os.geteuid())
        alias = self.base / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            runner.check_ancestors(alias, os.geteuid())

    def test_tree_scan_refuses_symlink_and_bind_mount_before_deleting_anything(self):
        outside = self.base / "outside"
        outside.write_text("唯一原件")
        (self.root / "alias").symlink_to(outside)
        with patch.object(runner, "mount_id", return_value=1):
            with self.assertRaises(ValueError):
                runner.inspect_tree(self.descriptor, self.identity["device"], 1, set())
        self.assertEqual(outside.read_text(), "唯一原件")
        self.assertTrue((self.root / "alias").is_symlink())
        with patch.object(runner, "mount_id", return_value=2):
            with self.assertRaisesRegex(ValueError, "挂载"):
                runner.remove_contents(self.descriptor, self.identity["device"], 1)
        self.assertEqual(outside.read_text(), "唯一原件")

    def test_live_adopted_descendant_keeps_directory(self):
        records = {}
        child = proc(999, os.getpid(), 42)
        with (patch.object(runner, "check_ancestors"),
              patch.object(runner, "reap_adopted"),
              patch.object(runner, "process_table", return_value=({999: child}, []))):
            result = runner.cleanup(self.root, self.descriptor, self.identity, records, lambda: None)
        self.assertFalse(result["cleanup_ready"])
        self.assertTrue(self.root.exists())
        self.assertEqual(records[(999, 42)], child)

    def test_proc_visibility_or_reference_failure_keeps_directory(self):
        for table, references in ((({}, [77]), []), (({}, []), ["proc_reference:77"]),
                                  (({}, []), ["proc_reference_visibility_incomplete:77"])):
            with self.subTest(table=table, references=references):
                with (patch.object(runner, "check_ancestors"),
                      patch.object(runner, "reap_adopted"),
                      patch.object(runner, "process_table", return_value=table),
                      patch.object(runner, "mount_id", return_value=1),
                      patch.object(runner, "referenced_by_proc", return_value=references)):
                    result = runner.cleanup(self.root, self.descriptor, self.identity, {}, lambda: None)
                self.assertFalse(result["cleanup_ready"])
                self.assertTrue(self.root.exists())

    def test_replaced_root_cannot_be_cleaned(self):
        previous = self.base / "previous"
        self.root.rename(previous)
        self.root.mkdir(mode=0o700)
        with patch.object(runner, "check_ancestors"):
            with self.assertRaises(ValueError):
                runner.cleanup(self.root, self.descriptor, self.identity, {}, lambda: None)
        self.assertTrue(previous.exists())
        self.assertTrue(self.root.exists())

    def test_complete_evidence_removes_only_owned_tree(self):
        (self.root / "nested").mkdir()
        (self.root / "nested" / "file").write_text("临时夹具")
        outside = self.base / "keep"
        outside.write_text("必须保留")
        with (patch.object(runner, "check_ancestors"),
              patch.object(runner, "reap_adopted"),
              patch.object(runner, "process_table", return_value=({}, [])),
              patch.object(runner, "mount_id", return_value=1),
              patch.object(runner, "referenced_by_proc", return_value=[])):
            archived = []
            result = runner.cleanup(self.root, self.descriptor, self.identity, {},
                                    lambda: archived.append(self.root.exists()))
        self.assertTrue(result["cleanup_ready"])
        self.assertEqual(archived, [True])
        self.assertFalse(self.root.exists())
        self.assertEqual(outside.read_text(), "必须保留")

    def test_cleanup_error_preserves_command_exit_and_only_child_tmpdir_changes(self):
        receipt = self.base / "receipt.json"
        child = MagicMock(pid=901, returncode=7)
        child.poll.return_value = 7
        source_environment = dict(os.environ)

        def root(created):
            created(self.root)
            return self.root, os.dup(self.descriptor), self.identity

        with (patch.object(runner.sys, "platform", "linux"),
              patch.object(runner, "create_root", side_effect=root),
              patch.object(runner, "process", side_effect=lambda pid: proc(pid, 0)),
              patch.object(runner, "process_table", return_value=({}, [])),
              patch.object(runner.ctypes, "CDLL") as library,
              patch.object(runner.subprocess, "Popen", return_value=child) as popen,
              patch.object(runner, "cleanup", side_effect=PermissionError)):
            library.return_value.prctl.return_value = 0
            code = runner.execute([sys.executable, "-c", "pass"], receipt)
        self.assertEqual(code, 7)
        self.assertEqual(os.environ, source_environment)
        self.assertEqual(popen.call_args.args[0], [sys.executable, "-c", "pass"])
        self.assertEqual(popen.call_args.kwargs, {"env": dict(source_environment, TMPDIR=str(self.root))})
        evidence = json.loads(receipt.read_text())
        self.assertEqual(evidence["command_exit_code"], 7)
        self.assertFalse(evidence["cleanup_ready"])
        self.assertEqual(evidence["status"], "retained")
        self.assertEqual(len(evidence["source_sha256"]), 64)
        self.assertEqual(len(evidence["argv_sha256"]), 64)
        self.assertTrue(evidence["source_unchanged"])
        self.assertTrue(self.root.exists())

    def test_non_linux_or_missing_argv_and_existing_receipt_are_rejected(self):
        receipt = self.base / "receipt.json"
        with patch.object(runner.sys, "platform", "darwin"):
            with self.assertRaises(ValueError):
                runner.execute([sys.executable], receipt)
        self.assertFalse(receipt.exists())
        with patch.object(runner.sys, "platform", "linux"):
            with self.assertRaises(ValueError):
                runner.execute([], receipt)
            receipt.write_text("旧证据")
            with self.assertRaises(FileExistsError):
                runner.execute([sys.executable], receipt)
        self.assertEqual(receipt.read_text(), "旧证据")


if __name__ == "__main__":
    unittest.main()
