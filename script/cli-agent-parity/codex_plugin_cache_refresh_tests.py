"""持久来源探针的创建与回收回归；Windows 真 Job 测试不调用模型或用户配置。"""

import json
import os
from pathlib import Path
import queue
import stat
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
        recorder.readers = {name: Mock() for name in ('stdout', 'stderr')}
        for reader in recorder.readers.values():
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
                recorder.readers['stdout'].is_alive.return_value = reader_alive
                with self.assertRaises(ValueError):
                    recorder.close()
                self.assertFalse(recorder.trace['cleanup_confirmed'])
                recorder.job.close.assert_called_once()

    def test_real_base_recorder_channel_map_closes_both_streams_after_eof(self):
        # 使用另一个脚本的真实父类及合成 Python 子进程，防止 mock 掩盖线程容器变更。
        with tempfile.TemporaryDirectory(prefix='cache-refresh-contract-') as temporary:
            directory = Path(temporary)
            env = {key: value for key, value in os.environ.items()
                   if key.upper() in ('SYSTEMROOT', 'WINDIR', 'PATH', 'TMP', 'TEMP')}
            child = ('import json,sys;print(json.dumps({"channel":"stdout"}),flush=True);'
                     'print(json.dumps({"channel":"stderr"}),file=sys.stderr,flush=True);sys.stdin.read()')
            trace, events = {}, []
            recorder = probe.CacheRefreshRecorder([sys.executable, '-c', child], env, directory, events, trace)
            recorder.close()
            self.assertEqual(set(recorder.readers), {'stdout', 'stderr'})
            self.assertTrue(all(not reader.is_alive() for reader in recorder.readers.values()))
            self.assertTrue(all(state['eof'] for state in recorder.reader_states.values()))
            self.assertEqual({event['channel'] for event in events}, {'stdout', 'stderr'})
            self.assertTrue(trace['root_exited_naturally'])
            self.assertEqual(trace['root_exit_code'], 0)
            self.assertTrue(trace['readers_eof'])
            self.assertTrue(trace['cleanup_confirmed'])
            self.assertTrue(recorder.process.stdout.closed)
            self.assertTrue(recorder.process.stderr.closed)

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

    def test_readonly_cleanup_requires_complete_job_and_eof_evidence(self):
        complete = {'assigned_before_resume': True, 'cleanup_confirmed': True,
                    'job_active_after_cleanup': 0, 'readers_eof': True, 'reader_errors': [],
                    'root_exited_naturally': True, 'root_exit_code': 0}
        self.assertTrue(probe.windows_cleanup_proven({'app_server_traces': [{'process_cleanup': complete}]}))
        self.assertFalse(probe.windows_cleanup_proven({}))
        for key, value in (('assigned_before_resume', False), ('cleanup_confirmed', False),
                           ('job_active_after_cleanup', 1), ('job_active_after_cleanup', False),
                           ('readers_eof', False), ('reader_errors', ['failure']),
                           ('root_exited_naturally', False), ('root_exit_code', 1)):
            with self.subTest(key=key, value=value):
                self.assertFalse(probe.windows_cleanup_proven({'app_server_traces': [
                    {'process_cleanup': complete}, {'process_cleanup': complete | {key: value}}]}))
        for key in ('close_failure', 'job_close_failure', 'stream_close_failures'):
            trace = {'process_cleanup': complete.copy()}
            if key == 'close_failure':
                trace[key] = {'type': 'OSError'}
            else:
                trace['process_cleanup'][key] = {'type': 'OSError'}
            with self.subTest(key=key):
                self.assertFalse(probe.windows_cleanup_proven({'app_server_traces': [trace]}))

    def test_cleanup_diagnostics_preserve_original_error_when_inspection_fails(self):
        report = {}
        original = PermissionError('original removal failure')
        handler = probe.cleanup_error_handler(Path('private'), None, report)
        with patch.object(Path, 'lstat', side_effect=FileNotFoundError('concurrent removal')):
            with self.assertRaises(PermissionError) as raised:
                handler(os.unlink, 'private/file', (PermissionError, original, None))
        self.assertIs(raised.exception, original)
        diagnostic = report['directory_cleanup_errors'][0]
        self.assertEqual(diagnostic['original_error']['message'], 'original removal failure')
        self.assertEqual(diagnostic['retry_rejected']['type'], 'FileNotFoundError')
        self.assertFalse(diagnostic['readonly_retry_attempted'])

    def windows_empty_job_report(self, directory):
        trace = {}
        env = {key: value for key, value in os.environ.items()
               if key.upper() in ('SYSTEMROOT', 'WINDIR', 'PATH', 'TMP', 'TEMP')}
        recorder = probe.CacheRefreshRecorder([sys.executable, '-c', 'import sys;sys.stdin.read()'],
                                              env, directory, [], trace)
        recorder.close()
        self.assertTrue(probe.windows_cleanup_proven({'app_server_traces': [{'process_cleanup': trace}]}))
        return {'app_server_traces': [{'process_cleanup': trace}]}

    @unittest.skipUnless(os.name == 'nt', '需要真实 Windows 只读属性和 Job 清理证据')
    def test_windows_readonly_file_is_removed_after_real_job_and_eof_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve() / 'private'
            directory.mkdir()
            target = directory / 'tmp_pack_fixture'
            target.write_bytes(b'private git pack')
            os.chmod(target, stat.S_IREAD)
            self.assertTrue(target.stat().st_file_attributes & 1)
            report = self.windows_empty_job_report(directory)
            with patch.object(probe, 'run'):
                probe.run_and_cleanup(Path('codex'), directory, report)
            self.assertFalse(directory.exists())
            self.assertTrue(report['private_directory_removed'])
            self.assertEqual(len(report['directory_cleanup_errors']), 1)
            diagnostic = report['directory_cleanup_errors'][0]
            self.assertEqual(diagnostic['original_error']['winerror'], 5)
            self.assertTrue(diagnostic['file_attributes'] & 1)
            self.assertTrue(diagnostic['readonly_retry_attempted'])
            self.assertTrue(diagnostic['readonly_retry_succeeded'])

    @unittest.skipUnless(os.name == 'nt', '需要真实 Windows 文件占用错误')
    def test_windows_nonreadonly_sharing_error_is_not_retried_or_suppressed(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve() / 'private'
            directory.mkdir()
            target = directory / 'locked-pack'
            report = self.windows_empty_job_report(directory)
            with target.open('wb') as handle:
                handle.write(b'held by the test, outside the finished Job')
                handle.flush()
                self.assertFalse(target.stat().st_file_attributes & 1)
                with patch.object(probe, 'run'):
                    with self.assertRaises(PermissionError):
                        probe.run_and_cleanup(Path('codex'), directory, report)
            self.assertTrue(target.exists())
            self.assertFalse(report['private_directory_removed'])
            diagnostic = report['directory_cleanup_errors'][0]
            self.assertFalse(diagnostic['readonly_retry_attempted'])
            self.assertFalse(diagnostic['file_attributes'] & 1)
            self.assertEqual(report['cleanup_failure']['winerror'], diagnostic['original_error']['winerror'])

    @unittest.skipUnless(os.name == 'nt', '需要真实 Windows 文件属性')
    def test_windows_nonreadonly_access_denied_keeps_original_error_and_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve()
            target = directory / 'ordinary-file'
            target.write_bytes(b'preserved')
            report = self.windows_empty_job_report(directory)
            original = PermissionError(13, 'injected access denied', str(target))
            original.winerror = 5
            handler = probe.cleanup_error_handler(directory, probe.file_identity(directory.lstat()), report)
            with self.assertRaises(PermissionError) as raised:
                handler(os.unlink, str(target), (PermissionError, original, None))
            self.assertIs(raised.exception, original)
            self.assertEqual(target.read_bytes(), b'preserved')
            self.assertFalse(report['directory_cleanup_errors'][0]['readonly_retry_attempted'])

    @unittest.skipUnless(os.name == 'nt', '需要真实 Windows 只读属性与文件占用错误')
    def test_windows_readonly_retry_failure_restores_attribute_and_preserves_first_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve() / 'private'
            directory.mkdir()
            target = directory / 'locked-readonly-pack'
            report = self.windows_empty_job_report(directory)
            with target.open('wb') as handle:
                handle.write(b'preserved')
                handle.flush()
                os.chmod(target, stat.S_IREAD)
                original = PermissionError(13, 'injected initial readonly denial', str(target))
                original.winerror = 5
                handler = probe.cleanup_error_handler(directory, probe.file_identity(directory.lstat()), report)
                # 固定第一次错误次序；重试执行真实 unlink，当前测试持有的句柄必须阻止删除。
                with patch.object(probe.os, 'unlink', wraps=os.unlink) as unlink:
                    with self.assertRaises(PermissionError) as raised:
                        handler(unlink, str(target), (PermissionError, original, None))
                    unlink.assert_called_once_with(target)
                self.assertIs(raised.exception, original)
            self.assertTrue(target.stat().st_file_attributes & 1)
            diagnostic = report['directory_cleanup_errors'][0]
            self.assertTrue(diagnostic['readonly_retry_attempted'])
            self.assertFalse(diagnostic['readonly_retry_succeeded'])
            self.assertTrue(diagnostic['original_readonly_restored'])
            self.assertIn('retry_failure', diagnostic)
            os.chmod(target, stat.S_IWRITE)

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


class BackgroundRefreshWaitTests(unittest.TestCase):
    def files(self, home, cache_matches, revision_matches):
        cache = home / 'private-cache'
        cache.mkdir()
        (cache / 'private-name').write_bytes(b'upstream')
        expected = probe.tree_hashes(cache)
        if not cache_matches:
            (cache / 'private-name').write_bytes(b'patched')
        config = '[marketplaces.codex-warp]\n'
        if revision_matches:
            config += f'last_revision = "{probe.PLUGIN_COMMIT}"\n'
        (home / 'config.toml').write_text(config, encoding='utf-8')
        return cache, expected

    def listing(self, home, cache, expected, report, ticks, on_sleep=None):
        recorder = Mock()
        recorder.rpc.side_effect = [{'codexHome': str(home)}, {'data': []}]
        with patch.object(probe, 'CacheRefreshRecorder', return_value=recorder), \
                patch.object(probe.time, 'monotonic_ns', side_effect=ticks), \
                patch.object(probe.time, 'sleep', side_effect=on_sleep):
            try:
                return probe.native_listing(Path('codex'), {'CODEX_HOME': str(home)}, home, report,
                                            'cache_only_restart', cache, expected_revert=expected)
            finally:
                recorder.close.assert_called_once()

    def test_native_refresh_can_finish_after_twenty_seconds_without_changing_success_conditions(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            cache, expected = self.files(home, False, True)
            report = {'app_server_traces': []}

            def finish_refresh(delay):
                self.assertEqual(delay, probe.BACKGROUND_REFRESH_POLL_SECONDS)
                (cache / 'private-name').write_bytes(b'upstream')

            self.listing(home, cache, expected, report, [0, 0, 25_000_000_000, 25_000_000_019], finish_refresh)
            trace = report['app_server_traces'][0]
            self.assertTrue(trace['background_reverted_to_upstream'])
            wait = trace['background_refresh_wait']
            self.assertEqual(wait['timeout_seconds'], 100)
            self.assertEqual(wait['elapsed_ns'], 25_000_000_019)
            self.assertTrue(wait['final_observation']['cache_matches_expected'])
            self.assertEqual(wait['final_observation']['revision'], probe.PLUGIN_COMMIT)
            self.assertEqual(trace['background_revision_provenance'], 'config_last_revision')

    def test_timeout_keeps_diagnostics_when_only_tree_or_revision_matches(self):
        for cache_matches, revision_matches in ((True, False), (False, True)):
            with self.subTest(cache_matches=cache_matches), tempfile.TemporaryDirectory() as temporary:
                home = Path(temporary)
                cache, expected = self.files(home, cache_matches, revision_matches)
                report = {'app_server_traces': []}
                with self.assertRaisesRegex(ValueError, '没有在期限内复现真实后台还原'):
                    self.listing(home, cache, expected, report, [0, 0, 100_000_000_000, 100_000_000_023])
                trace = report['app_server_traces'][0]
                self.assertNotIn('background_reverted_to_upstream', trace)
                if revision_matches:
                    self.assertEqual(trace['background_revision_published'], probe.PLUGIN_COMMIT)
                else:
                    self.assertNotIn('background_revision_published', trace)
                wait = trace['background_refresh_wait']
                self.assertEqual(wait['elapsed_ns'], 100_000_000_023)
                observed = wait['final_observation']
                self.assertEqual(observed['cache_matches_expected'], cache_matches)
                self.assertEqual(observed['revision_matches_expected'], revision_matches)
                self.assertEqual(observed['cache_file_count'], 1)
                self.assertEqual(len(observed['cache_tree_sha256']), 64)
                self.assertNotIn(str(home), json.dumps(wait))
                self.assertNotIn('private-name', json.dumps(wait))

    def test_snapshot_after_deadline_cannot_reclassify_timeout_as_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            cache, expected = self.files(home, False, True)
            report = {'app_server_traces': []}
            with self.assertRaisesRegex(ValueError, '没有在期限内复现真实后台还原'):
                self.listing(home, cache, expected, report, [0, 0, 100_000_000_000, 100_000_000_001],
                             lambda delay: (cache / 'private-name').write_bytes(b'upstream'))
            trace = report['app_server_traces'][0]
            self.assertNotIn('background_reverted_to_upstream', trace)
            observed = trace['background_refresh_wait']['final_observation']
            self.assertTrue(observed['cache_matches_expected'])
            self.assertTrue(observed['revision_matches_expected'])

    def test_final_read_errors_are_safe_and_do_not_replace_original_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            cache, expected = self.files(home, True, True)
            secret = 'private-path-and-secret'
            primary = PermissionError(13, secret, str(cache))
            primary.winerror = 5
            report = {'app_server_traces': []}
            with patch.object(probe, 'tree_hashes', side_effect=primary):
                with self.assertRaises(PermissionError) as raised:
                    self.listing(home, cache, expected, report, [0, 0, 17, 18])
            self.assertIs(raised.exception, primary)
            wait = report['app_server_traces'][0]['background_refresh_wait']
            self.assertEqual(wait['elapsed_ns'], 18)
            observed = wait['final_observation']
            self.assertIsNone(observed['cache_tree_sha256'])
            self.assertEqual(observed['cache_read_error'], {'type': 'PermissionError', 'errno': 13, 'winerror': 5})
            self.assertTrue(observed['revision_matches_expected'])
            self.assertNotIn(secret, json.dumps(wait))
            self.assertNotIn(str(home), json.dumps(wait))

    def test_invalid_revision_is_not_copied_into_diagnostics(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            cache, expected = self.files(home, True, False)
            secret = 'private-path-and-secret'
            (home / 'config.toml').write_text(f'[marketplaces.codex-warp]\nlast_revision = "{secret}"\n',
                                            encoding='utf-8')
            observed = probe.background_refresh_snapshot(cache, expected, home)
            self.assertIsNone(observed['revision'])
            self.assertEqual(observed['revision_state'], 'invalid')
            self.assertFalse(observed['revision_matches_expected'])
            self.assertNotIn(secret, json.dumps(observed))

    def test_current_releases_use_exact_installed_marketplace_metadata(self):
        for version in ('0.155.1', '0.156.1'):
            with tempfile.TemporaryDirectory() as temporary:
                home = Path(temporary)
                cache, expected = self.files(home, True, False)
                marketplace = home / 'marketplace'
                marketplace.mkdir()
                metadata = {'source_type': 'git', 'source': probe.UPSTREAM_URL,
                            'ref_name': probe.PLUGIN_COMMIT, 'sparse_paths': [],
                            'revision': probe.PLUGIN_COMMIT}
                (marketplace / '.codex-marketplace-install.json').write_text(
                    json.dumps(metadata), encoding='utf-8')
                self.assertTrue(probe.background_refresh_published(
                    cache, expected, home, marketplace, version))
                observed = probe.background_refresh_snapshot(
                    cache, expected, home, marketplace, version)
                self.assertEqual(observed['revision_provenance'], 'installed_marketplace_metadata')
                self.assertTrue(observed['marketplace_contract_matches_expected'])
                metadata['source'] = 'https://example.invalid/replaced.git'
                (marketplace / '.codex-marketplace-install.json').write_text(
                    json.dumps(metadata), encoding='utf-8')
                self.assertFalse(probe.background_refresh_published(
                    cache, expected, home, marketplace, version))

    def test_unverified_version_cannot_infer_marketplace_metadata_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            for version in ('0.156.2', '0.156.1-alpha', ''):
                with self.subTest(version=version), self.assertRaisesRegex(ValueError, '受控来源'):
                    probe.marketplace_revision_evidence(home, home, version)


if __name__ == '__main__':
    unittest.main()
