"""持久来源探针的创建与回收回归；Windows 真 Job 测试不调用模型或用户配置。"""

import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
import probe_codex_plugin_cache_refresh as probe


class CacheRefreshCleanupTests(unittest.TestCase):
    def creation(self):
        command = ['C:\\固定 路径\\codex.exe', 'app-server', '--stdio']
        env = {'CODEX_HOME': 'private'}
        directory = Path('private')
        api = SimpleNamespace(CreateProcess=Mock(return_value=(100, 101, 200, 201)), CloseHandle=Mock())
        job = Mock()
        arguments = (None, subprocess.list2cmdline(command), None, None, True, 0x800,
                     env, os.fspath(directory), object())
        return api, job, command, env, directory, arguments

    def test_suspended_spawn_preserves_exact_arguments_and_restores_api(self):
        api, job, command, env, directory, arguments = self.creation()
        original, trace = api.CreateProcess, {}
        with probe.suspended_creation(api, job, command, env, directory, trace):
            self.assertEqual(api.CreateProcess(*arguments), (100, 101, 200, 201))
        self.assertIs(api.CreateProcess, original)
        expected = list(arguments)
        expected[5] |= 4
        original.assert_called_once_with(*expected)
        job.attach_and_resume.assert_called_once_with(100, 101)
        self.assertEqual(trace, {'pid': 200, 'assigned_before_resume': True})
        api.CloseHandle.assert_not_called()

    def test_mismatched_argv_environment_or_directory_is_rejected_before_creation(self):
        for index, replacement in ((1, 'different'), (6, {'CODEX_HOME': 'private'}), (7, 'other')):
            with self.subTest(index=index):
                api, job, command, env, directory, arguments = self.creation()
                original = api.CreateProcess
                changed = list(arguments)
                changed[index] = replacement
                with self.assertRaisesRegex(ValueError, '不匹配'):
                    with probe.suspended_creation(api, job, command, env, directory, {}):
                        api.CreateProcess(*changed)
                original.assert_not_called()
                self.assertIs(api.CreateProcess, original)

    def test_other_thread_does_not_enter_private_job(self):
        api, job, command, env, directory, arguments = self.creation()
        original = api.CreateProcess
        with probe.suspended_creation(api, job, command, env, directory, {}):
            thread = threading.Thread(target=lambda: api.CreateProcess(*arguments))
            thread.start()
            thread.join(timeout=2)
            self.assertFalse(thread.is_alive())
            job.attach_and_resume.assert_not_called()
            api.CreateProcess(*arguments)
        self.assertEqual(original.call_args_list[0].args[5], 0x800)
        self.assertEqual(original.call_args_list[1].args[5], 0x804)
        job.attach_and_resume.assert_called_once()

    def test_assignment_or_resume_failure_reclaims_owned_suspended_handles(self):
        for cleanup_fails in (False, True):
            with self.subTest(cleanup_fails=cleanup_fails):
                api, job, command, env, directory, arguments = self.creation()
                original, trace = api.CreateProcess, {}
                primary = OSError('assign/resume denied')
                job.attach_and_resume.side_effect = primary
                if cleanup_fails:
                    job.reclaim_suspended.side_effect = OSError('terminate denied')
                with self.assertRaises(OSError) as raised:
                    with probe.suspended_creation(api, job, command, env, directory, trace):
                        api.CreateProcess(*arguments)
                self.assertIs(raised.exception, primary)
                self.assertIs(api.CreateProcess, original)
                job.reclaim_suspended.assert_called_once_with(100)
                self.assertEqual([call.args for call in api.CloseHandle.call_args_list], [(101,), (100,)])
                self.assertEqual(trace.get('suspended_process_exit_confirmed', False), not cleanup_fails)
                self.assertEqual('startup_cleanup_failure' in trace, cleanup_fails)

    def recorder(self):
        recorder = probe.CacheRefreshRecorder.__new__(probe.CacheRefreshRecorder)
        recorder.trace = {}
        recorder.process = Mock(returncode=0)
        recorder.readers = [Mock(), Mock()]
        for reader in recorder.readers:
            reader.is_alive.return_value = False
        recorder.reader_errors = []
        recorder.job = Mock()
        return recorder

    def test_root_success_does_not_hide_forced_descendant_cleanup(self):
        recorder = self.recorder()
        recorder.job.wait_empty.side_effect = [1, 0]
        recorder.close()
        self.assertTrue(recorder.trace['root_exited_naturally'])
        self.assertTrue(recorder.trace['descendants_forced'])
        self.assertTrue(recorder.trace['readers_eof'])
        self.assertTrue(recorder.trace['cleanup_confirmed'])
        self.assertEqual(recorder.trace['job_active_after_cleanup'], 0)
        recorder.job.terminate.assert_called_once()
        recorder.job.close.assert_called_once()

    def test_job_zero_never_makes_forced_root_a_natural_success(self):
        recorder = self.recorder()
        recorder.process.wait.side_effect = [subprocess.TimeoutExpired('fixture', 5), 1]
        recorder.process.returncode = 1
        recorder.job.wait_empty.return_value = 0
        with self.assertRaisesRegex(ValueError, '没有自然正常退出'):
            recorder.close()
        self.assertFalse(recorder.trace['root_exited_naturally'])
        self.assertTrue(recorder.trace['root_forced'])
        self.assertEqual(recorder.trace['root_exit_code'], 1)
        self.assertTrue(recorder.trace['cleanup_confirmed'])

    def test_remaining_job_or_live_reader_cannot_be_confirmed(self):
        for active, reader_alive in ((1, False), (0, True)):
            with self.subTest(active=active, reader_alive=reader_alive):
                recorder = self.recorder()
                recorder.job.wait_empty.return_value = active
                recorder.readers[0].is_alive.return_value = reader_alive
                with self.assertRaises(ValueError):
                    recorder.close()
                self.assertFalse(recorder.trace['cleanup_confirmed'])
                recorder.job.close.assert_called_once()

    def test_protocol_failure_survives_close_failure(self):
        recorder = Mock()
        primary = RuntimeError('first RPC failure')
        recorder.rpc.side_effect = primary
        recorder.close.side_effect = ValueError('reader still alive')
        report = {'app_server_traces': []}
        with patch.object(probe, 'CacheRefreshRecorder', return_value=recorder):
            with self.assertRaises(RuntimeError) as raised:
                probe.native_listing(Path('codex'), {'CODEX_HOME': 'private'}, Path('private'), report,
                                     'fixture', Path('cache'))
        self.assertIs(raised.exception, primary)
        self.assertEqual(report['app_server_traces'][0]['close_failure']['type'], 'ValueError')

    def test_failure_preserves_private_directory_and_original_error(self):
        report = {}
        primary = ValueError('reader did not reach EOF')
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with patch.object(probe, 'run', side_effect=primary), patch.object(probe.shutil, 'rmtree') as cleanup:
                with self.assertRaises(ValueError) as raised:
                    probe.run_and_cleanup(Path('codex'), directory, report)
                cleanup.assert_not_called()
            self.assertIs(raised.exception, primary)
            self.assertFalse(report['private_directory_removed'])
            self.assertEqual(report['failure']['message'], str(primary))
            self.assertTrue(directory.exists())

    def test_cleanup_failure_after_success_is_still_failure(self):
        report = {}
        with patch.object(probe, 'run'), patch.object(probe.shutil, 'rmtree', side_effect=PermissionError('pack locked')):
            with self.assertRaises(PermissionError):
                probe.run_and_cleanup(Path('codex'), Path('private'), report)
        self.assertFalse(report['private_directory_removed'])
        self.assertEqual(report['cleanup_failure']['type'], 'PermissionError')

    def test_revert_requires_both_tree_and_native_revision(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            cache = home / 'cache'
            cache.mkdir()
            (cache / 'original').write_bytes(b'upstream')
            expected = probe.tree_hashes(cache)
            config = home / 'config.toml'
            config.write_text('[marketplaces.codex-warp]\n', encoding='utf-8')
            self.assertFalse(probe.background_refresh_published(cache, expected, home))
            config.write_text(f'[marketplaces.codex-warp]\nlast_revision = "{probe.PLUGIN_COMMIT}"\n', encoding='utf-8')
            self.assertTrue(probe.background_refresh_published(cache, expected, home))
            (cache / 'original').write_bytes(b'patched')
            self.assertFalse(probe.background_refresh_published(cache, expected, home))

    @unittest.skipUnless(os.name == 'nt', '需要真实 Windows Job 与继承的管道句柄')
    def test_windows_real_job_reclaims_inherited_pipe_and_locked_file_after_root_eof(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            locked = directory / 'private-pack'
            child = ('import json,os,sys,time;f=open(sys.argv[1],"wb");'
                     'f.write(b"private");f.flush();print(json.dumps({"child_pid":os.getpid()}),flush=True);time.sleep(60)')
            root = ('import subprocess,sys;subprocess.Popen([sys.executable,"-c",sys.argv[1],sys.argv[2]],'
                    'stdin=subprocess.DEVNULL,stdout=sys.stdout,stderr=sys.stderr);sys.stdin.read()')
            env = {key: value for key, value in os.environ.items()
                   if key.upper() in ('SYSTEMROOT', 'WINDIR', 'PATH', 'TMP', 'TEMP')}
            trace, events = {}, []
            recorder = probe.CacheRefreshRecorder([sys.executable, '-c', root, child, str(locked)],
                                                  env, directory, events, trace)
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    try:
                        event = recorder.queue.get(timeout=0.1)
                    except queue.Empty:
                        continue
                    if isinstance(event, dict) and event.get('child_pid'):
                        break
                else:
                    self.fail(f'子进程没有就绪: {events}')
                with self.assertRaises(PermissionError):
                    locked.unlink()
            finally:
                recorder.close()
            self.assertTrue(trace['assigned_before_resume'])
            self.assertTrue(trace['root_exited_naturally'])
            self.assertEqual(trace['root_exit_code'], 0)
            self.assertTrue(trace['descendants_forced'])
            self.assertEqual(trace['job_active_after_cleanup'], 0)
            self.assertTrue(trace['readers_eof'])
            self.assertTrue(trace['cleanup_confirmed'])
            locked.unlink()


if __name__ == '__main__':
    unittest.main()
