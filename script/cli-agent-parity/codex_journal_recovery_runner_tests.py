"""只验证驱动的准备与一次性边界，不启动 Rust 或 CLI。"""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('driver', Path(__file__).with_name('run_codex_journal_recovery.py'))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class DriverBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='infinishell-journal-driver-unit-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.binary = self.root / 'never-executed-binary'
        self.manifest = self.root / 'manifest.json'
        self.output = self.root / 'report.json'
        self.binary.write_bytes(b'not a real executable\n')
        self.manifest.write_text('{}\n')

    def argv(self):
        return ['driver', '--execute', '--test-binary', str(self.binary),
                '--test-binary-sha256', hashlib.sha256(self.binary.read_bytes()).hexdigest(),
                '--source-manifest', str(self.manifest),
                '--source-manifest-sha256', hashlib.sha256(self.manifest.read_bytes()).hexdigest(),
                '--output', str(self.output)]

    def test_prepared_mode_never_spawns_or_reserves(self):
        with mock.patch.object(sys, 'argv', ['driver']), mock.patch.object(driver.subprocess, 'run') as run, mock.patch.object(driver.subprocess, 'Popen') as popen, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(driver.main(), 0)
        self.assertEqual(json.loads(output.getvalue())['rust_executed'], False)
        run.assert_not_called()
        popen.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_changed_bound_binary_is_rejected_before_reservation(self):
        args = self.argv()
        self.binary.write_bytes(b'changed\n')
        with mock.patch.object(sys, 'argv', args), mock.patch.object(driver.subprocess, 'run') as run, mock.patch.object(driver.subprocess, 'Popen') as popen:
            with self.assertRaisesRegex(ValueError, 'bound_material_mismatch'):
                driver.main()
        run.assert_not_called()
        popen.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_existing_reservation_is_never_reused(self):
        self.output.write_text('previous failure retained\n')
        with mock.patch.object(sys, 'argv', self.argv()), mock.patch.object(driver.subprocess, 'run') as run, mock.patch.object(driver.subprocess, 'Popen') as popen:
            with self.assertRaises(FileExistsError):
                driver.main()
        run.assert_not_called()
        popen.assert_not_called()
        self.assertEqual(self.output.read_text(), 'previous failure retained\n')

    def test_missing_exact_test_entry_retains_failure_without_starting_child(self):
        fixture = self.root / 'infinishell-codex-journal-crash-unit'
        fixture.mkdir()
        failed_list = mock.Mock(returncode=0, stdout=b'0 tests, 0 benchmarks\n')
        with mock.patch.object(sys, 'argv', self.argv()), mock.patch.object(driver.tempfile, 'mkdtemp', return_value=str(fixture)), mock.patch.object(driver.subprocess, 'run', return_value=failed_list) as run, mock.patch.object(driver.subprocess, 'Popen') as popen, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(driver.main(), 1)
        run.assert_called_once()
        popen.assert_not_called()
        report = json.loads(self.output.read_text())
        self.assertFalse(report['passed'])
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(len(report['rows']), 1)
        self.assertFalse(report['native_cli_executed'])


if __name__ == '__main__':
    unittest.main()
