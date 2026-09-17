"""固定输入和编码的独立回归；原生 Windows 执行由 probe 单独验证，不用 mock 冒充。"""

import base64
import hashlib
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
from codex_windows_hook_command import (PREFIX, SCRIPTS, cmd_line, encode, source_text,
                                       verify_encoding, verify_windows_argv, require_original_bytes)
from codex_windows_hook_inputs import (CODEX_COMMIT, RELEASE_ASSETS, fetch_file, plugin_base,
                                       regular_file, sha256, verify_codex, verify_plugin)
import probe_codex_windows_hooks as native_probe


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
            'app/assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-prompt-submit.sh').read_text(encoding='utf-8')
        candidate, evidence = native_probe.binary_query_candidate(original)
        self.assertEqual(candidate.replace('jq --binary -r', 'jq -r'), original)
        self.assertEqual(evidence['original_sha256'], hashlib.sha256(original.encode()).hexdigest())
        self.assertEqual(evidence['candidate_sha256'], hashlib.sha256(candidate.encode()).hexdigest())
        self.assertFalse(evidence['product_recipe_modified'])
        self.assertFalse(evidence['newline_normalization_applied'])
        with self.assertRaises(ValueError):
            native_probe.binary_query_candidate(candidate)

    def test_native_jq_binary_boundary_preserves_original_lf_crlf_and_literal_escapes(self):
        evidence = native_probe.verify_jq_newline_boundary(os.environ.copy())
        self.assertEqual([case['input_line_ending'] for case in evidence['cases']], ['LF', 'CRLF'])
        for case, prompt in zip(evidence['cases'], native_probe.HOOK_PROMPTS):
            self.assertEqual(bytes.fromhex(case['outputs']['binary']['stdout_hex']), prompt.encode('utf-8') + b'\n')
        self.assertFalse(evidence['normalization_used'])


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
