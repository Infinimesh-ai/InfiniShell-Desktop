"""ConPTY 探针的离线判定回归；不加载或执行 Windows 二进制。"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from probe_codex_windows_conpty import (WinApi, environment_block, notifications,
                                         read_shell_diagnostics, require_candidate_contract,
                                         shell_diagnostic_source, verify_transport)


def fixture_case(prompt='中文输入\nEnglish'):
    return {'name': '隔离 中文', 'passed': True, 'markers': {
        'on-session-start.sh': {'session_id': 'native-session-unique', 'cwd': 'C:/隔离 空格'},
        'on-prompt-submit.sh': {'turn_id': 'native-turn-unique', 'cwd': 'C:/隔离 空格', 'prompt': prompt}}}


def osc(event, session='native-session-unique', turn='native-turn-unique', terminator=b'\x07', query='中文输入\nEnglish'):
    payload = {'v': 1, 'agent': 'codex', 'event': event, 'session_id': session,
               'turn_id': turn, 'cwd': 'C:/隔离 空格', 'query': query}
    return b'\x1b]777;notify;warp://cli-agent;' + json.dumps(payload, ensure_ascii=False).encode('utf-8') + terminator


class ConptyProbeTests(unittest.TestCase):
    def test_exact_native_session_and_turn_are_required(self):
        raw = b'ordinary terminal output' + osc('session_start') + osc('prompt_submit', terminator=b'\x1b\\')
        found = verify_transport(raw, [fixture_case()])
        self.assertEqual(len(found), 2)
        self.assertEqual(found[1]['cwd'], 'C:/隔离 空格')
        self.assertEqual(len(notifications(raw)), 2)

    def test_correct_ids_cannot_hide_corrupted_unicode_directory(self):
        raw = (osc('session_start') + osc('prompt_submit')).replace('隔离'.encode(), b'??')
        with self.assertRaises(ValueError):
            verify_transport(raw, [fixture_case()])

    def test_query_must_preserve_lf_crlf_and_literal_escape_without_normalization(self):
        for prompt in ('中文\nEnglish', '中文\r\nEnglish', 'literal \\r\\n'):
            with self.subTest(prompt=repr(prompt)):
                raw = osc('session_start') + osc('prompt_submit', query=prompt)
                self.assertEqual(verify_transport(raw, [fixture_case(prompt)])[1]['query'], prompt)
        for original, changed in (('中文\nEnglish', '中文\r\nEnglish'),
                                  ('中文\r\nEnglish', '中文\nEnglish'),
                                  ('literal \\r\\n', 'literal \r\n')):
            with self.subTest(original=repr(original)), self.assertRaises(ValueError):
                verify_transport(osc('session_start') + osc('prompt_submit', query=changed), [fixture_case(original)])

    def test_native_hook_success_without_osc_is_failure(self):
        with self.assertRaises(ValueError):
            verify_transport(b'completed sessionStart userPromptSubmit', [fixture_case()])

    def test_conout_diagnostic_cannot_satisfy_native_transport(self):
        raw = b'\x1b]777;notify;warp://cli-agent;{"diagnostic_nonce":"synthetic"}\x07'
        with self.assertRaises(ValueError):
            verify_transport(raw, [fixture_case()])

    def test_old_session_or_old_turn_cannot_satisfy_native_transport(self):
        for raw in (osc('session_start', session='old') + osc('prompt_submit'),
                    osc('session_start') + osc('prompt_submit', turn='old')):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                verify_transport(raw, [fixture_case()])

    def test_duplicate_notification_and_native_failure_are_rejected(self):
        raw = osc('session_start') + osc('prompt_submit')
        with self.assertRaises(ValueError):
            verify_transport(raw + osc('prompt_submit'), [fixture_case()])
        case = fixture_case()
        case['passed'] = False
        with self.assertRaises(ValueError):
            verify_transport(raw, [case])
        with self.assertRaises(ValueError):
            verify_transport(raw, [])

    def test_partial_or_invalid_payload_is_not_success(self):
        self.assertEqual(notifications(osc('session_start')[:-1]), [])
        with self.assertRaises(json.JSONDecodeError):
            notifications(b'\x1b]777;notify;warp://cli-agent;broken\x07')
        with self.assertRaises(UnicodeDecodeError):
            notifications(b'\x1b]777;notify;warp://cli-agent;\xff\x07')

    def test_environment_has_double_terminator_and_preserves_unicode(self):
        value = environment_block({'z': '中文 空格', 'PATH': 'C:\\bin', 'SystemRoot': 'C:\\Windows'})
        self.assertEqual(value, 'PATH=C:\\bin\0SystemRoot=C:\\Windows\0z=中文 空格\0\0')
        for env in ({'PATH': 'a', 'Path': 'b'}, {'A=B': 'bad'}, {'A': 'bad\0value'}):
            with self.subTest(env=env), self.assertRaises(ValueError):
                environment_block(env)

    def test_candidate_really_inherits_console_and_standard_handles(self):
        self.assertEqual(len(require_candidate_contract()), 64)

    def test_private_bash_observer_preserves_hook_stdin_and_standard_output(self):
        bash = os.environ.get('INFINISHELL_GIT_BASH_EXE') or shutil.which('bash')
        jq = os.environ.get('INFINISHELL_JQ_EXE') or shutil.which('jq')
        if os.name == 'nt':
            candidates = [Path(bash)] if bash else []
            candidates += [Path(os.environ.get('PROGRAMFILES', '')) / 'Git/usr/bin/bash.exe',
                           Path(os.environ.get('LOCALAPPDATA', '')) / 'Programs/Git/usr/bin/bash.exe']
            bash = next((path for path in candidates if path.is_file()
                         and (path.parent / 'msys-2.0.dll').is_file()), None)
        self.assertTrue(bash and jq, '通知入口诊断测试要求实际 Bash 与 jq，缺依赖不能计通过')
        with tempfile.TemporaryDirectory(prefix='conpty-entry-') as temporary:
            root = Path(temporary) / '中文 空格'
            root.mkdir()
            observer = root / 'observe.sh'
            observer.write_text(shell_diagnostic_source(), encoding='utf-8', newline='\n')
            (root / 'should-use-structured.sh').write_text(
                'should_use_structured() { [ -n "${WARP_CLI_AGENT_PROTOCOL_VERSION:-}" ] && '
                '[ -n "${WARP_CLIENT_VERSION:-}" ]; }\n', encoding='utf-8', newline='\n')
            script = root / 'warp-notify.sh'
            script.write_text('IFS= read -r input\nprintf "stdout:%s\\n" "$input"\n'
                              'printf "stderr:unchanged\\n" >&2\n', encoding='utf-8', newline='\n')
            payload = {'v': 1, 'agent': 'codex', 'event': 'session_start', 'session_id': 'diagnostic-only'}
            environment = {**os.environ, 'BASH_ENV': observer.as_posix(),
                'INFINISHELL_CONPTY_DIAGNOSTICS_DIR': root.as_posix(),
                'INFINISHELL_HOOK_PROBE_CASE': root.as_posix(),
                'WARP_CLI_AGENT_PROTOCOL_VERSION': '1', 'WARP_CLIENT_VERSION': 'test',
                'PATH': os.pathsep.join((str(Path(bash).parent), str(Path(jq).parent),
                                         os.environ.get('PATH', '')))}
            result = subprocess.run([str(bash), '--noprofile', '--norc', '--', script.as_posix(),
                'warp://cli-agent', json.dumps(payload)], input='中文 literal $() &\n',
                env=environment, capture_output=True, text=True, encoding='utf-8', timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.replace('\r\n', '\n'), 'stdout:中文 literal $() &\n')
            self.assertEqual(result.stderr.replace('\r\n', '\n'), 'stderr:unchanged\n')
            records = read_shell_diagnostics(root)
            self.assertEqual(len(records), 1)
            self.assertEqual(records[0]['structured_gate'], 'allowed')
            self.assertEqual(records[0]['notification']['session_id'], 'diagnostic-only')
            self.assertFalse(records[0]['stdin_tty'])
            self.assertFalse(records[0]['stdout_tty'])
            self.assertFalse(records[0]['stderr_tty'])
            # 诊断文件不是 HPCON 输出，不能补成通知运输成功。
            with self.assertRaises(ValueError):
                verify_transport(result.stdout.encode('utf-8'), [fixture_case()])

    def test_malformed_shell_diagnostic_is_not_accepted_as_observation(self):
        with tempfile.TemporaryDirectory(prefix='conpty-invalid-entry-') as temporary:
            root = Path(temporary)
            self.assertEqual(read_shell_diagnostics(root), [])
            (root / '1.json').write_text(json.dumps({'version': 1, 'structured_gate': 'allowed',
                'tty_open_status': '0', 'stdin_tty': False, 'stdout_tty': False,
                'stderr_tty': False}), encoding='utf-8')
            with self.assertRaises(ValueError):
                read_shell_diagnostics(root)

    @unittest.skipIf(os.name == 'nt', '此条只证明非 Windows 不能伪造原生 API 验收')
    def test_other_platforms_cannot_load_native_fixture(self):
        with self.assertRaises(ValueError):
            WinApi()


if __name__ == '__main__':
    unittest.main()
