"""固定输入和编码的独立回归；原生 Windows 执行由 probe 单独验证，不用 mock 冒充。"""

import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
from codex_windows_hook_command import (PREFIX, SCRIPTS, cmd_line, encode, source_text,
                                       verify_encoding, verify_windows_argv, require_original_bytes)
from codex_windows_hook_inputs import (CODEX_COMMIT, CODEX_VERSION, HOOK_CODEX_VERSIONS, RELEASE_ASSETS,
                                       codex_contract, fetch_file, obtain_inputs, plugin_base,
                                       regular_file, sha256, verify_codex, verify_plugin)
import probe_codex_windows_hooks as native_probe
import prepare_codex_cli as prepare
from codex_windows_formal import (EVENTS, authorize_config, exact_plugin_tree, formal_transport_expectations,
                                  probe_bundle, restore_config, split_trigger_hooks, validate_native_hooks)


class FormalResourceTests(unittest.TestCase):
    def fixtures(self):
        hooks = [{'key': 'test-only:' + event, 'eventName': event, 'handlerType': 'command',
                  'command': encode(script), 'timeoutSec': 600, 'source': 'plugin',
                  'pluginId': 'warp@codex-warp', 'currentHash': 'sha256:' + hashlib.sha256(event.encode()).hexdigest(),
                  'enabled': True, 'trustStatus': 'untrusted'} for event, script in zip(EVENTS, SCRIPTS)]
        blocker = dict(hooks[1], key='test-only:extra-blocker', command='test-only-blocker')
        return hooks, hooks + [blocker], blocker['command']

    def test_formal_and_old_candidate_have_separate_immutable_baselines(self):
        repo = Path(__file__).resolve().parents[2]
        current, replacements, source_hash = probe_bundle(repo, 'formal')
        old, old_replacements, old_source_hash = probe_bundle(repo, 'candidate')
        self.assertEqual((current['patch_revision'], old['patch_revision']), (5, 3))
        self.assertEqual(len(source_hash), 64)
        self.assertIsNone(old_source_hash)
        self.assertNotIn('scripts/on-prompt-submit.sh', old_replacements)
        self.assertNotEqual(replacements['scripts/warp-notify.sh'], old_replacements['scripts/warp-notify.sh'])
        plugin = repo / 'app/assets/bundled/cli-agent-plugins/codex/source/plugins/warp'
        self.assertEqual(len(exact_plugin_tree(plugin, current)), 10)
        with self.assertRaises(ValueError):
            exact_plugin_tree(plugin, old)
        with self.assertRaises(ValueError):
            probe_bundle(repo, 'unknown')

    def test_formal_full_tree_rejects_missing_extra_and_modified_files(self):
        repo = Path(__file__).resolve().parents[2]
        metadata, _, _ = probe_bundle(repo, 'formal')
        source = repo / 'app/assets/bundled/cli-agent-plugins/codex/source/plugins/warp'
        for change in ('missing', 'extra', 'modified'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                target = Path(temporary) / 'plugin'
                shutil.copytree(source, target)
                if change == 'missing':
                    (target / 'scripts/on-stop.sh').unlink()
                elif change == 'extra':
                    (target / 'test-marker.sh').write_text('changed')
                else:
                    (target / 'hooks/hooks.json').write_text('{}')
                with self.assertRaises(ValueError):
                    exact_plugin_tree(target, metadata)

    def test_old_candidate_is_reproducible_after_formal_resources_are_updated(self):
        repo = Path(__file__).resolve().parents[2]
        bundle = repo / 'app/assets/bundled/cli-agent-plugins/codex'
        metadata, replacements, _ = probe_bundle(repo, 'candidate')
        with tempfile.TemporaryDirectory() as temporary:
            plugin = Path(temporary) / 'plugin'
            shutil.copytree(bundle / 'source/plugins/warp', plugin)
            for name, contents in replacements.items():
                (plugin / name).write_bytes(contents)
            (plugin / 'scripts/on-prompt-submit.sh').write_bytes(
                (bundle / 'revisions/rev3/scripts/on-prompt-submit.sh').read_bytes())
            self.assertEqual(len(exact_plugin_tree(plugin, metadata)), 10)
            evidence = {}
            native_probe.instrument_candidate(plugin, evidence)
            self.assertTrue((plugin / 'scripts/on-prompt-submit.sh.fixture-original').is_file())
            self.assertIn('query_binary_candidate', evidence)
            self.assertIn('notification_transport_candidate', evidence)
            self.assertFalse(evidence['query_binary_candidate']['newline_normalization_applied'])
            # 二次变换必须失败，正式模式只能绕过候选变换，不能吞掉它的固定基线检查。
            with self.assertRaises(ValueError):
                native_probe.instrument_candidate(plugin, {})

    def test_sixth_hook_never_becomes_a_formal_registered_hash(self):
        formal, runtime, blocker = self.fixtures()
        result = split_trigger_hooks(formal, runtime, blocker)
        self.assertEqual(result['formal_hooks'], formal)
        self.assertEqual(result['test_only_blocker'], runtime[-1])
        self.assertEqual(result['native_events_verified'], [])
        self.assertEqual(result['native_events_not_verified'], list(EVENTS))
        for changed in (runtime[:-1], runtime + [runtime[-1]], formal + [dict(runtime[-1], key=formal[0]['key'])]):
            with self.subTest(count=len(changed)), self.assertRaises(ValueError):
                split_trigger_hooks(formal, changed, blocker)
        with self.assertRaises(ValueError):
            split_trigger_hooks(runtime, runtime, blocker)

    def test_blocker_cannot_hide_changed_formal_command_hash_or_timeout(self):
        for field, value in (('command', 'different'), ('currentHash', 'sha256:' + '0' * 64),
                             ('timeoutSec', 1), ('eventName', 'stop'), ('key', 'other')):
            formal, runtime, blocker = self.fixtures()
            runtime = json.loads(json.dumps(runtime))
            runtime[0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                split_trigger_hooks(formal, runtime, blocker)

    def test_registration_requires_native_windows_command_and_exact_private_cache(self):
        formal, _, _ = self.fixtures()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            plugin = root / 'source'
            cache = root / 'home/cache'
            for path in (plugin, cache):
                (path / 'hooks').mkdir(parents=True)
                (path / 'hooks/hooks.json').write_text('{}')
            for hook in formal:
                hook['sourcePath'] = str(cache / 'hooks/hooks.json')
            commands = {encode(script) for script in SCRIPTS}
            self.assertEqual(validate_native_hooks(formal, root / 'home', plugin, commands, 'untrusted'), cache)
            with self.assertRaises(ValueError):
                validate_native_hooks(formal, root / 'different-home', plugin, commands, 'untrusted')
            for field, value in (('trustStatus', 'trusted'), ('currentHash', 'unknown'),
                                 ('command', 'bash old-posix'), ('enabled', False), ('sourcePath', str(root / 'outside'))):
                changed = json.loads(json.dumps(formal))
                changed[0][field] = value
                with self.subTest(field=field), self.assertRaises(ValueError):
                    validate_native_hooks(changed, root / 'home', plugin, commands, 'untrusted')
            (cache / 'uncontrolled').write_text('extra')
            with self.assertRaises(ValueError):
                validate_native_hooks(formal, root / 'home', plugin, commands, 'untrusted')

    def test_private_trust_transaction_restores_exact_bytes_and_preserves_concurrent_edit(self):
        formal, _, _ = self.fixtures()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'config.toml'
            original = '# 用户原文\r\nmodel = "fixed"\r\n'.encode('utf-8')
            path.write_bytes(original)
            before, written = authorize_config(path, formal)
            self.assertEqual(before, original)
            self.assertTrue(written.startswith(original))
            receipt = {}
            restore_config(path, before, written, receipt)
            self.assertEqual(path.read_bytes(), original)
            self.assertTrue(receipt['restored'])
            before, written = authorize_config(path, formal)
            changed = written + b'# concurrent user edit\n'
            path.write_bytes(changed)
            receipt = {}
            with self.assertRaises(ValueError):
                restore_config(path, before, written, receipt)
            self.assertEqual(path.read_bytes(), changed)
            self.assertTrue(receipt['concurrent_change_detected'])
            self.assertFalse(receipt['restored'])

    def test_formal_expectations_preserve_original_text_without_marker(self):
        for prompt in native_probe.HOOK_PROMPTS:
            result = formal_transport_expectations(Path("C:/中文 ' $()"), 'session', 'turn', prompt)
            self.assertEqual(result['prompt'].encode(), prompt.encode())
            self.assertFalse(result['query_normalization_applied'])
            self.assertEqual(result['provenance'], 'app_server_request_response')

    def test_cleanup_requires_success_eof_and_rollback_and_never_swallows_permission_error(self):
        def case():
            return {'passed': True, 'mode': 'candidate', 'config_rollback': {'restored': True},
                    'process_closes': [{'root_exited_naturally': True, 'root_exit_code': 0,
                                        'output_readers_eof': True} for _ in range(2)]}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'owned-private'
            root.mkdir()
            (root / 'fixture').write_text('private')
            for field in ('passed', 'eof', 'rollback'):
                changed = case()
                if field == 'passed':
                    changed['passed'] = False
                elif field == 'eof':
                    changed['process_closes'][0]['output_readers_eof'] = False
                else:
                    changed['config_rollback']['restored'] = False
                receipt = {}
                with self.subTest(field=field), self.assertRaises(ValueError):
                    native_probe.cleanup_private_cases(root, [changed], receipt)
                self.assertFalse(receipt['attempted'])
                self.assertTrue(root.is_dir())
            failure = PermissionError(13, 'test-only permission failure', str(root / 'fixture'))
            receipt = {}
            with patch.object(native_probe.shutil, 'rmtree', side_effect=failure), self.assertRaises(PermissionError) as caught:
                native_probe.cleanup_private_cases(root, [case()], receipt)
            self.assertIs(caught.exception, failure)
            self.assertEqual(receipt['failure']['filename'], str(root / 'fixture'))
            self.assertFalse(receipt['deleted'])
            self.assertTrue(root.is_dir())
            receipt = {}
            native_probe.cleanup_private_cases(root, [case()], receipt)
            self.assertTrue(receipt['deleted'])
            self.assertFalse(root.exists())

    def test_cleanup_removes_only_confirmed_private_windows_readonly_file(self):
        case = {'passed': True, 'mode': 'candidate', 'config_rollback': {'restored': True},
                'process_closes': [{'root_exited_naturally': True, 'root_exit_code': 0,
                                    'output_readers_eof': True} for _ in range(2)]}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'owned-private'
            target = root / '.git/objects/pack/tmp_idx_test'
            target.parent.mkdir(parents=True)
            target.write_text('private Git object')
            original_lstat, original_rmtree = Path.lstat, shutil.rmtree
            target_info = original_lstat(target)

            def metadata(path, *args, **kwargs):
                if path == target:
                    return SimpleNamespace(st_mode=target_info.st_mode, st_nlink=1,
                                           st_file_attributes=stat.FILE_ATTRIBUTE_READONLY)
                return original_lstat(path, *args, **kwargs)

            def denied_once(directory, onerror):
                error = PermissionError(13, 'read-only private file', str(target))
                error.winerror = 5
                onerror(os.unlink, str(target), (PermissionError, error, None))
                original_rmtree(directory)

            receipt = {}
            with patch.object(native_probe.sys, 'platform', 'win32'), \
                    patch.object(Path, 'lstat', metadata), \
                    patch.object(native_probe.shutil, 'rmtree', side_effect=denied_once), \
                    patch.object(native_probe.os, 'chmod') as chmod:
                native_probe.cleanup_private_cases(root, [case], receipt)
            chmod.assert_called_once_with(target, target_info.st_mode | stat.S_IWRITE, follow_symlinks=False)
            self.assertTrue(receipt['deleted'])
            self.assertEqual(receipt['readonly_files_removed'], 1)
            self.assertFalse(root.exists())

    def test_cleanup_readonly_retry_rejects_escape_reparse_hardlink_and_other_errors(self):
        case = {'passed': True, 'mode': 'candidate', 'config_rollback': {'restored': True},
                'process_closes': [{'root_exited_naturally': True, 'root_exit_code': 0,
                                    'output_readers_eof': True} for _ in range(2)]}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'owned-private'
            target = root / 'objects/private-file'
            target.parent.mkdir(parents=True)
            target.write_text('private')
            original_lstat = Path.lstat
            for boundary in ('outside', 'sharing_violation', 'no_readonly', 'file_reparse',
                             'parent_reparse', 'hardlink', 'directory', 'symlink'):
                with self.subTest(boundary=boundary):
                    error = PermissionError(13, 'must remain rejected', str(target))
                    error.winerror = 32 if boundary == 'sharing_violation' else 5

                    def metadata(path, *args, **kwargs):
                        if path == target:
                            mode = stat.S_IFDIR if boundary == 'directory' else (
                                stat.S_IFLNK if boundary == 'symlink' else stat.S_IFREG)
                            return SimpleNamespace(st_mode=mode, st_nlink=2 if boundary == 'hardlink' else 1,
                                st_file_attributes=(0 if boundary == 'no_readonly' else stat.FILE_ATTRIBUTE_READONLY)
                                | (stat.FILE_ATTRIBUTE_REPARSE_POINT if boundary == 'file_reparse' else 0))
                        if path == target.parent and boundary == 'parent_reparse':
                            return SimpleNamespace(st_mode=stat.S_IFDIR,
                                                   st_file_attributes=stat.FILE_ATTRIBUTE_REPARSE_POINT)
                        return original_lstat(path, *args, **kwargs)

                    def denied(directory, onerror):
                        name = root.parent / 'outside' if boundary == 'outside' else target
                        onerror(os.unlink, str(name), (PermissionError, error, None))

                    receipt = {}
                    with patch.object(native_probe.sys, 'platform', 'win32'), \
                            patch.object(Path, 'lstat', metadata), \
                            patch.object(native_probe.shutil, 'rmtree', side_effect=denied), \
                            patch.object(native_probe.os, 'chmod') as chmod, \
                            self.assertRaises(PermissionError) as caught:
                        native_probe.cleanup_private_cases(root, [case], receipt)
                    self.assertIs(caught.exception, error)
                    chmod.assert_not_called()
                    self.assertFalse(receipt['deleted'])
                    self.assertEqual(receipt['readonly_files_removed'], 0)
                    self.assertEqual(target.read_text(), 'private')

    def test_close_receipt_requires_actual_process_exit_and_both_output_readers(self):
        with tempfile.TemporaryDirectory() as temporary:
            events, evidence = [], {}
            recorder = native_probe.NativeRecorder([sys.executable, '-c',
                'import sys; sys.stdin.read(); print("{}")'], os.environ.copy(), Path(temporary), events)
            native_probe.close_case(recorder, evidence)
            receipt = evidence['process_closes'][0]
            self.assertTrue(receipt['root_exited_naturally'])
            self.assertTrue(receipt['output_readers_eof'])
            self.assertEqual(receipt['root_exit_code'], 0)
            self.assertFalse(receipt['all_descendants_job_verified'])


class NativeCompletionTests(unittest.TestCase):
    def fixtures(self):
        expected = {
            2: {'eventName': 'sessionStart', 'sourcePath': 'private/hooks.json', 'expected_status': 'completed'},
            3: {'eventName': 'userPromptSubmit', 'sourcePath': 'private/hooks.json', 'expected_status': 'completed'},
            4: {'eventName': 'userPromptSubmit', 'sourcePath': 'private/hooks.json', 'expected_status': 'stopped'},
        }
        hooks = [dict(item, displayOrder=order, command='blocker' if order == 4 else 'notification')
                 for order, item in expected.items()]
        events = []
        for order, item in expected.items():
            for method, status in (('hook/started', 'running'), ('hook/completed', item['expected_status'])):
                events.append({'channel': 'stdout', 'value': {'method': method,
                    'params': {'threadId': 'session', 'turnId': 'turn', 'run': {
                        'id': f'run-{order}', 'displayOrder': order, 'status': status,
                        'eventName': item['eventName'], 'sourcePath': item['sourcePath']}}}})
        events.append({'channel': 'stdout', 'value': {'method': 'turn/completed',
            'params': {'threadId': 'session', 'turn': {'id': 'turn', 'status': 'completed', 'error': None}}}})
        return expected, hooks, events

    def test_requires_all_original_hooks_blocker_and_actual_turn_completion(self):
        expected, _, events = self.fixtures()
        result = native_probe.hook_completion_snapshot(events, 'session', 'turn', expected)
        self.assertTrue(result['complete'])
        self.assertEqual(len(result['completed']), 3)
        for missing in (0, 1, 3, 5, 6):
            with self.subTest(missing=missing):
                result = native_probe.hook_completion_snapshot(events[:missing] + events[missing + 1:], 'session', 'turn', expected)
                self.assertFalse(result['complete'])

    def test_old_events_and_stderr_json_cannot_complete_current_hooks(self):
        expected, _, events = self.fixtures()
        result = native_probe.hook_completion_snapshot(events, 'new-session', 'turn', expected)
        self.assertFalse(result['complete'])
        self.assertEqual(result['ignored_unrelated_events'], 7)
        for event in events:
            event['channel'] = 'stderr'
        self.assertFalse(native_probe.hook_completion_snapshot(events, 'session', 'turn', expected)['complete'])

    def test_duplicate_events_are_idempotent_but_conflicting_runs_fail(self):
        expected, _, events = self.fixtures()
        self.assertTrue(native_probe.hook_completion_snapshot(events + events, 'session', 'turn', expected)['complete'])
        changed = json.loads(json.dumps(events[1]))
        changed['value']['params']['run']['id'] = 'different-execution'
        with self.assertRaises(ValueError):
            native_probe.hook_completion_snapshot(events + [changed], 'session', 'turn', expected)
        events[3]['value']['params']['run']['status'] = 'failed'
        with self.assertRaises(ValueError):
            native_probe.hook_completion_snapshot(events, 'session', 'turn', expected)

    def test_completion_after_old_ten_second_limit_is_still_bounded_and_verified(self):
        _, hooks, events = self.fixtures()
        clock = SimpleNamespace(now=0)
        recorder = SimpleNamespace(events=[], events_changed=Mock(), process=Mock(), reader_errors=[])
        recorder.process.poll.return_value = None
        def wait(timeout):
            clock.now += 4
            if clock.now >= 12:
                recorder.events.extend(events)
        recorder.events_changed.wait.side_effect = wait
        evidence = {}
        with patch.object(native_probe.time, 'monotonic', side_effect=lambda: clock.now):
            native_probe.wait_for_hook_completion(recorder, 'session', 'turn', hooks, 'blocker', [], evidence)
        self.assertTrue(evidence['hook_completion_wait']['complete'])
        self.assertEqual(evidence['hook_completion_wait']['elapsed_ms'], 12000)
        self.assertEqual(evidence['hook_completion_wait']['timeout_seconds'], 45)

    def test_timeout_reader_failure_exit_and_model_request_never_pass(self):
        _, hooks, _ = self.fixtures()
        for mode in ('timeout', 'reader', 'exit', 'model'):
            clock = SimpleNamespace(now=0)
            recorder = SimpleNamespace(events=[], events_changed=Mock(), process=Mock(), reader_errors=[])
            recorder.process.poll.return_value = None
            requests, evidence = [], {}
            def wait(timeout):
                clock.now += 5
            recorder.events_changed.wait.side_effect = wait
            if mode == 'reader':
                recorder.reader_errors.append('invalid UTF-8')
            if mode == 'exit':
                recorder.process.poll.return_value = 0
            if mode == 'model':
                requests.append({'method': 'POST', 'path': '/v1/responses'})
            with self.subTest(mode=mode), patch.object(native_probe.time, 'monotonic', side_effect=lambda: clock.now):
                with self.assertRaises(ValueError):
                    native_probe.wait_for_hook_completion(recorder, 'session', 'turn', hooks, 'blocker', requests, evidence)
            self.assertFalse(evidence['hook_completion_wait']['complete'])
            self.assertIn('failure', evidence['hook_completion_wait'])
            self.assertEqual(evidence['hook_completion_wait']['pending_display_orders'], [2, 3, 4])

    def test_binary_query_candidate_changes_only_the_jq_output_mode(self):
        original = (Path(__file__).resolve().parents[2] /
            'app/assets/bundled/cli-agent-plugins/codex/revisions/rev3/scripts/on-prompt-submit.sh').read_text(encoding='utf-8')
        candidate, evidence = native_probe.binary_query_candidate(original)
        self.assertEqual(candidate.replace('jq --binary -r', 'jq -r'), original)
        self.assertEqual(evidence['original_sha256'], hashlib.sha256(original.encode()).hexdigest())
        self.assertEqual(evidence['candidate_sha256'], hashlib.sha256(candidate.encode()).hexdigest())
        self.assertFalse(evidence['product_recipe_modified'])
        self.assertFalse(evidence['newline_normalization_applied'])
        with self.assertRaises(ValueError):
            native_probe.binary_query_candidate(candidate)

    @unittest.skipUnless(os.name == 'nt', '只验证 Windows 原生 jq 的 --binary 输出；Unix jq 无需支持该参数')
    def test_native_jq_binary_boundary_preserves_original_lf_crlf_and_literal_escapes(self):
        evidence = native_probe.verify_jq_newline_boundary(os.environ.copy())
        self.assertEqual([case['input_line_ending'] for case in evidence['cases']], ['LF', 'CRLF'])
        for case, prompt in zip(evidence['cases'], native_probe.HOOK_PROMPTS):
            self.assertEqual(bytes.fromhex(case['outputs']['binary']['stdout_hex']), prompt.encode('utf-8') + b'\n')
        self.assertFalse(evidence['normalization_used'])


class NativeRecorderCloseTests(unittest.TestCase):
    def inherited_writer(self, root, channels, delay):
        release = root / 'release-child'
        child_code = (
            'import json, pathlib, sys, time\n'
            'deadline = time.monotonic() + float(sys.argv[1])\n'
            'while time.monotonic() < deadline and not pathlib.Path(sys.argv[2]).exists():\n'
            '    time.sleep(0.01)\n'
            'for channel in json.loads(sys.argv[3]):\n'
            '    print(json.dumps({"child_done": channel}), file=getattr(sys, channel), flush=True)\n'
        )
        parent_code = (
            'import subprocess, sys\n'
            'sys.stdin.read()\n'
            f'subprocess.Popen([sys.executable, "-c", {child_code!r}, {str(delay)!r}, '
            f'{str(release)!r}, {json.dumps(channels)!r}], stdin=subprocess.DEVNULL, '
            f'stdout={"None" if "stdout" in channels else "subprocess.DEVNULL"}, '
            f'stderr={"None" if "stderr" in channels else "subprocess.DEVNULL"})\n'
        )
        recorder = native_probe.NativeRecorder([sys.executable, '-c', parent_code],
                                               os.environ.copy(), root, [])
        return recorder, release

    def release_and_drain(self, recorder, release):
        # 只释放夹具自己创建的短命后代；失败回执保持收尾超时时的原始快照。
        release.write_text('release', encoding='utf-8')
        for reader in recorder.readers.values():
            reader.join(timeout=5)
            self.assertFalse(reader.is_alive(), '测试后代未按释放信号退出')
        for stream in (recorder.process.stdout, recorder.process.stderr):
            stream.close()

    def test_close_waits_for_delayed_inherited_stdout_and_drains_final_line(self):
        with tempfile.TemporaryDirectory() as temporary:
            recorder, release = self.inherited_writer(Path(temporary), ['stdout'], 2.2)
            evidence = {}
            try:
                native_probe.close_case(recorder, evidence)
                receipt = evidence['process_closes'][0]
                self.assertTrue(receipt['root_exited_naturally'])
                self.assertEqual(receipt['root_exit_code'], 0)
                self.assertTrue(receipt['output_readers_eof'])
                self.assertEqual(receipt['output_eof_timeout_seconds'], 5)
                self.assertGreaterEqual(receipt['output_readers']['stdout']['finished_elapsed_ms'], 2000)
                self.assertEqual(receipt['pending_eof_channels'], [])
                self.assertEqual(receipt['alive_reader_channels'], [])
                self.assertTrue(all(state['eof'] for state in receipt['output_readers'].values()))
                self.assertTrue(any(event['value'] == {'child_done': 'stdout'} for event in recorder.events))
                self.assertFalse(receipt['all_descendants_job_verified'])
            finally:
                self.release_and_drain(recorder, release)

    def test_inherited_pipes_past_deadline_fail_with_both_channels_and_frozen_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            recorder, release = self.inherited_writer(Path(temporary), ['stdout', 'stderr'], 5)
            evidence = {}
            try:
                with patch.object(native_probe, 'OUTPUT_EOF_TIMEOUT', 0.15), self.assertRaisesRegex(
                        ValueError, 'stdout, stderr'):
                    native_probe.close_case(recorder, evidence)
                receipt = evidence['failed_close_receipt']
                self.assertTrue(receipt['root_exited_naturally'])
                self.assertEqual(receipt['root_exit_code'], 0)
                self.assertFalse(receipt['termination_requested'])
                self.assertFalse(receipt['kill_requested'])
                self.assertFalse(receipt['output_readers_eof'])
                self.assertFalse(receipt['all_descendants_job_verified'])
                self.assertEqual(receipt['pending_eof_channels'], ['stdout', 'stderr'])
                self.assertEqual(receipt['alive_reader_channels'], ['stdout', 'stderr'])
                self.assertEqual(receipt['output_eof_timeout_seconds'], 0.15)
                self.assertNotIn('process_closes', evidence)
            finally:
                self.release_and_drain(recorder, release)
            self.assertTrue(all(state['eof'] for state in recorder.reader_states.values()))
            self.assertFalse(any(state['eof'] for state in receipt['output_readers'].values()))

    def test_decoder_error_identifies_channel_without_counting_reader_exit_as_eof(self):
        with tempfile.TemporaryDirectory() as temporary:
            recorder = native_probe.NativeRecorder([sys.executable, '-c',
                'import os, sys; sys.stdin.read(); os.write(2, bytes([255]))'],
                os.environ.copy(), Path(temporary), [])
            evidence = {}
            with self.assertRaisesRegex(ValueError, '原生输出读取失败'):
                native_probe.close_case(recorder, evidence)
            receipt = evidence['failed_close_receipt']
            self.assertFalse(receipt['output_readers_eof'])
            self.assertEqual(receipt['pending_eof_channels'], ['stderr'])
            self.assertEqual(receipt['alive_reader_channels'], [])
            self.assertEqual(receipt['reader_errors'], [{'channel': 'stderr', 'type': 'UnicodeDecodeError'}])


class CommandTests(unittest.TestCase):
    def test_each_command_roundtrips_exact_fixed_source(self):
        for script in SCRIPTS:
            with self.subTest(script=script):
                command = encode(script)
                decoded = base64.b64decode(command[len(PREFIX):], validate=True).decode('utf-16le')
                self.assertEqual(decoded, "$NotificationHook = '" + script + "'\n" + source_text())
                self.assertLessEqual(len(command), 8000)
        self.assertTrue(verify_encoding()['encoding_roundtrip'])

    def test_platform_checkout_newlines_generate_identical_commands(self):
        plain = source_text()
        with patch.object(Path, 'read_text', return_value=plain.replace('\n', '\r\n')):
            self.assertEqual(source_text(), plain)

    def test_script_name_cannot_be_shell_source(self):
        for name in ('../on-stop.sh', "on-stop.sh'; whoami; '", 'on-stop.sh\n', 'unknown.sh'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                encode(name)

    def test_overlong_fixed_source_is_rejected(self):
        with patch('codex_windows_hook_command.source_text', return_value=' ' * 9000):
            with self.assertRaises(ValueError):
                encode(SCRIPTS[0])

    def test_cmd_boundary_accounts_for_utf16_and_executable(self):
        executable = Path('C:/含😀空格/Windows/System32/cmd.exe')
        command = encode(SCRIPTS[0])
        count = len(cmd_line(command, executable).encode('utf-16le')) // 2
        boundary = command + ' ' * (8191 - count)
        self.assertEqual(len(cmd_line(boundary, executable).encode('utf-16le')) // 2, 8191)
        with self.assertRaises(ValueError):
            cmd_line(boundary + ' ', executable)

    def test_encoded_command_contains_no_environment_interpolation(self):
        for script in SCRIPTS:
            command = encode(script)
            self.assertTrue(command.isascii())
            self.assertFalse(any(character in command for character in '%!$`&^()'))

    def test_native_argv_failure_preserves_fixed_expected_and_actual_values(self):
        observed = {}

        def collapsed_result(plain, environment):
            values = json.loads(Path(environment['PROBE_VALUES']).read_text(encoding='utf-8'))
            observed['expected'] = values
            observed['actual'] = [' '.join(values)]
            Path(environment['PROBE_OUTPUT']).write_text(json.dumps(observed['actual']), encoding='utf-8')

        # 模拟旧 PS 数组转换的合并结果，只验证失败证据；不冒充 Windows 进程实测。
        with patch('codex_windows_hook_command.run_powershell', side_effect=collapsed_result):
            with self.assertRaises(ValueError) as failure:
                verify_windows_argv({})
        self.assertEqual(json.loads(str(failure.exception).split(': ', 1)[1]), observed)
        self.assertEqual(observed['expected'][0], '')
        self.assertEqual(len(observed['expected']), 8)

    def test_native_argv_validation_preserves_empty_and_quoted_fixture_values(self):
        def unchanged_result(plain, environment):
            values = json.loads(Path(environment['PROBE_VALUES']).read_text(encoding='utf-8'))
            Path(environment['PROBE_OUTPUT']).write_text(json.dumps(values), encoding='utf-8')

        with patch('codex_windows_hook_command.run_powershell', side_effect=unchanged_result):
            verify_windows_argv({})

    def test_raw_bytes_failure_keeps_both_streams_and_missing_capture_distinct(self):
        payload = '中文\\nEnglish\r\n'.encode('utf-8')
        for captured, stdout in [(b'\xef\xbb\xbf' + payload, b'native-bytes-ok'),
                                 (payload, b'native-bytes-ok\r\n'), (None, b'')]:
            with self.subTest(captured=captured, stdout=stdout):
                completed = subprocess.CompletedProcess([], 0, stdout=stdout, stderr=b'fixture stderr')
                with self.assertRaises(ValueError) as failure:
                    require_original_bytes(completed, captured, payload, 'fixture-entry')
                evidence = json.loads(str(failure.exception).split(': ', 1)[1])
                self.assertEqual(evidence['case'], 'fixture-entry')
                self.assertEqual(base64.b64decode(evidence['expected_stdin_base64']), payload)
                self.assertEqual(base64.b64decode(evidence['actual_stdout_base64']), stdout)
                self.assertEqual(base64.b64decode(evidence['stderr_base64']), b'fixture stderr')
                self.assertEqual(None if captured is None else base64.b64decode(evidence['actual_stdin_base64']), captured)
        require_original_bytes(subprocess.CompletedProcess([], 0, stdout=b'native-bytes-ok', stderr=b''), payload, payload, 'fixture-entry')


class FixedInputTests(unittest.TestCase):
    def test_hook_versions_keep_legacy_default_and_bind_fixed_current_release(self):
        self.assertEqual(CODEX_VERSION, '0.147.0')
        self.assertEqual(HOOK_CODEX_VERSIONS, ('0.147.0', '0.156.1'))
        self.assertEqual(codex_contract('0.147.0')['commit'], CODEX_COMMIT)
        self.assertEqual(codex_contract('0.156.1')['commit'], 'b412ff32c417f855c2b2d1581b77058eed87c84b')
        package = prepare.packages_for_version('0.156.1')['windows-x64']
        self.assertEqual(package['files'][package['entrypoint']][1],
                         '70bcb05f9bf1a4e7306edd0cd1b57d02af3267ad02a34b26f45c8c4bb20a3301')
        for version in ('latest', '0.155.1', '0.156.2', ''):
            with self.subTest(version=version), self.assertRaises(ValueError):
                codex_contract(version)

    def test_current_executable_requires_complete_verified_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / 'bin').mkdir(mode=0o700)
            package = {'target': 'x86_64-pc-windows-msvc', 'entrypoint': 'bin/codex.exe',
                       'directories': {'bin': 0o755}, 'archive': 'fixture.tar.gz',
                       'bytes': 1, 'sha256': '0' * 64,
                       'metadata': {'version': '0.156.1', 'target': 'x86_64-pc-windows-msvc',
                                    'entrypoint': 'bin/codex.exe'}, 'files': {}}
            bodies = {'bin/codex.exe': b'fixture-executable', 'bin/helper.dll': b'fixture-dll',
                      'codex-package.json': json.dumps(package['metadata']).encode()}
            for name, body in bodies.items():
                member = root / name
                member.write_bytes(body)
                member.chmod(0o600)
                package['files'][name] = (len(body), hashlib.sha256(body).hexdigest(), 0o644)
            executable = root / 'bin/codex.exe'
            with patch.object(prepare, 'packages_for_version', return_value={'windows-x64': package}):
                evidence = verify_codex(executable, 'x86_64', '0.156.1')
                self.assertTrue(evidence['runtime_tree_verified'])
                self.assertEqual(evidence['version'], '0.156.1')
                for invalid in ('missing', 'modified', 'extra'):
                    with self.subTest(invalid=invalid):
                        member = root / 'bin/helper.dll'
                        if invalid == 'missing':
                            member.unlink()
                        elif invalid == 'modified':
                            member.write_bytes(b'changed-dll')
                        else:
                            (root / 'bin/extra.dll').write_bytes(b'extra')
                        with self.assertRaises(ValueError):
                            verify_codex(executable, 'x86_64', '0.156.1')
                        member.write_bytes(bodies['bin/helper.dll'])
                        member.chmod(0o600)
                        (root / 'bin/extra.dll').unlink(missing_ok=True)
            with self.assertRaises(ValueError):
                verify_codex(executable, 'x86_64', '0.147.0')

    def test_current_download_uses_fixed_complete_package_preparer(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve()
            executable = directory / 'runtime-0.156.1-windows-x64/bin/codex.exe'
            plugin = directory / 'plugin'
            package = prepare.packages_for_version('0.156.1')['windows-x64']
            with patch('codex_windows_hook_inputs.plugin_base', return_value={'tree_sha256': {}}), \
                 patch('codex_windows_hook_inputs.verify_plugin', return_value={}), \
                 patch('codex_windows_hook_inputs.verify_codex', return_value={'version': '0.156.1'}) as verify, \
                 patch.object(prepare, 'fetch_file') as fetch, \
                 patch.object(prepare, 'extract_runtime_package', return_value=executable) as extract:
                actual, _, _ = obtain_inputs(directory, directory, 'x86_64', plugin=plugin, version='0.156.1')
                archive = directory / ('0.156.1-' + package['archive'])
                fetch.assert_called_once_with(
                    'https://github.com/openai/codex/releases/download/rust-v0.156.1/' + package['archive'],
                    archive, package['sha256'], package['bytes'])
                extract.assert_called_once_with(archive, directory / 'runtime-0.156.1-windows-x64',
                                                'windows-x64', '0.156.1')
                verify.assert_called_once_with(executable, 'x86_64', '0.156.1')
                self.assertEqual(actual, executable)

    def test_fixed_native_version_mismatch_is_rejected_before_hook_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for version in HOOK_CODEX_VERSIONS:
                wrong = '0.156.1' if version == CODEX_VERSION else CODEX_VERSION
                with patch.object(prepare.subprocess, 'run', return_value=SimpleNamespace(stdout=f'codex-cli {wrong}')) as run:
                    with self.assertRaises(ValueError):
                        native_probe.verified_version(root / 'codex.exe', root, version)
                    environment = run.call_args.kwargs['env']
                    self.assertTrue(Path(environment['CODEX_HOME']).is_relative_to(root))
                    self.assertTrue(Path(environment['HOME']).is_relative_to(root))

    def test_probe_cli_exposes_only_explicit_fixed_hook_versions(self):
        for name in ('probe_codex_windows_hooks.py', 'probe_codex_windows_conpty.py'):
            script = Path(__file__).with_name(name)
            environment = dict(os.environ, PYTHONIOENCODING='utf-8')
            help_result = subprocess.run([sys.executable, '-B', str(script), '--help'],
                                         env=environment, capture_output=True, text=True, encoding='utf-8', timeout=10)
            self.assertEqual(help_result.returncode, 0)
            self.assertIn('--codex-version {0.147.0,0.156.1}', help_result.stdout)
            invalid = subprocess.run([sys.executable, '-B', str(script), '--codex-version', 'latest'],
                                     env=environment, capture_output=True, text=True, encoding='utf-8', timeout=10)
            self.assertEqual(invalid.returncode, 2)
            self.assertIn('invalid choice', invalid.stderr)

    def test_official_pins_are_specific_release_assets(self):
        self.assertEqual(len(CODEX_COMMIT), 40)
        for architecture, (name, size, digest) in RELEASE_ASSETS.items():
            self.assertEqual(name, f'codex-{architecture}-pc-windows-msvc.exe')
            self.assertGreater(size, 200_000_000)
            self.assertEqual(len(bytes.fromhex(digest)), 32)
        self.assertEqual(len(plugin_base(Path(__file__).resolve().parents[2])['tree_sha256']), 10)

    def test_external_executable_cannot_pass_by_version_string(self):
        with tempfile.TemporaryDirectory() as tmp:
            executable = Path(tmp) / 'codex.exe'
            executable.write_bytes(b'codex-cli 0.147.0')
            with self.assertRaises(ValueError):
                verify_codex(executable, 'x86_64')

    def test_raw_tree_rejects_modified_missing_and_extra_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / 'scripts/hook.sh'
            path.parent.mkdir()
            contents = b'#!/bin/bash\nprintf original\n'
            path.write_bytes(contents)
            base = {'tree_sha256': {'scripts/hook.sh': hashlib.sha256(contents).hexdigest()}}
            self.assertEqual(verify_plugin(root, base), base['tree_sha256'])
            path.write_bytes(contents + b'changed')
            with self.assertRaises(ValueError):
                verify_plugin(root, base)
            path.write_bytes(contents)
            extra = root / 'custom.txt'
            extra.touch()
            with self.assertRaises(ValueError):
                verify_plugin(root, base)
            extra.unlink()
            path.unlink()
            with self.assertRaises(ValueError):
                verify_plugin(root, base)

    def test_hardlinked_input_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'input'
            path.write_bytes(b'fixed')
            os.link(path, Path(tmp) / 'alias')
            with self.assertRaises(ValueError):
                regular_file(path)

    def test_existing_download_is_verified_without_network_or_overwrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'fixed'
            path.write_bytes(b'fixed')
            digest = sha256(path)
            url = 'https://github.com/openai/codex/releases/download/rust-v0.147.0/test.exe'
            with patch('urllib.request.urlopen', side_effect=AssertionError('不应联网')):
                fetch_file(url, path, digest, 5)
                with self.assertRaises(ValueError):
                    fetch_file(url, path, '0' * 64, 5)
            self.assertEqual(path.read_bytes(), b'fixed')

    def test_latest_or_uncontrolled_download_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            for url in ('https://github.com/openai/codex/releases/latest/download/codex.exe',
                        'http://example.com/codex.exe'):
                with self.subTest(url=url), self.assertRaises(ValueError):
                    fetch_file(url, Path(tmp) / 'exe', '0' * 64)


if __name__ == '__main__':
    unittest.main()
