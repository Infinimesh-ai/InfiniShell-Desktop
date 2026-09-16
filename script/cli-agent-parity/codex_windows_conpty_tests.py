"""ConPTY 探针的离线判定回归；不加载或执行 Windows 二进制。"""

import json
import os
import unittest

from probe_codex_windows_conpty import (WinApi, environment_block, notifications,
                                         require_candidate_contract, verify_transport)


def fixture_case():
    return {'name': '隔离 中文', 'passed': True, 'markers': {
        'on-session-start.sh': {'session_id': 'native-session-unique'},
        'on-prompt-submit.sh': {'turn_id': 'native-turn-unique'}}}


def osc(event, session='native-session-unique', turn='native-turn-unique', terminator=b'\x07'):
    payload = {'v': 1, 'agent': 'codex', 'event': event, 'session_id': session,
               'turn_id': turn, 'cwd': 'C:/隔离 空格'}
    return b'\x1b]777;notify;warp://cli-agent;' + json.dumps(payload, ensure_ascii=False).encode('utf-8') + terminator


class ConptyProbeTests(unittest.TestCase):
    def test_exact_native_session_and_turn_are_required(self):
        raw = b'ordinary terminal output' + osc('session_start') + osc('prompt_submit', terminator=b'\x1b\\')
        found = verify_transport(raw, [fixture_case()])
        self.assertEqual(len(found), 2)
        self.assertEqual(found[1]['cwd'], 'C:/隔离 空格')
        self.assertEqual(len(notifications(raw)), 2)

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

    @unittest.skipIf(os.name == 'nt', '此条只证明非 Windows 不能伪造原生 API 验收')
    def test_other_platforms_cannot_load_native_fixture(self):
        with self.assertRaises(ValueError):
            WinApi()


if __name__ == '__main__':
    unittest.main()
