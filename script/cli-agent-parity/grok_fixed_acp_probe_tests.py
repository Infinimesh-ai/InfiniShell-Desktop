#!/usr/bin/env python3
"""固定 Grok ACP 边界的离线回归；真实 Python 子进程只验证 I/O 与清理，不冒充 Grok。"""

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import grok_fixed_acp_probe as probe
from probe_claude_no_credentials_tests import python_fixture_environment


def native_responses():
    fixture = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/grok-1.0.30-handshake.ndjson"
    rows = [json.loads(line) for line in fixture.read_text(encoding="utf-8").splitlines()]
    return {row["message"]["id"]: row["message"] for row in rows
            if row["direction"] == "stdout" and "id" in row["message"]}


def transcript():
    expected = probe.requests(Path("isolated-project"), "current-probe")
    responses = native_responses()
    records = []
    native_id = 0
    for request in expected:
        records.append({"direction": "stdin", "message": request})
        if "id" in request:
            native_id += 1
            response = copy.deepcopy(responses[native_id])
            response["id"] = request["id"]
            records.append({"direction": "stdout", "message": response})
    return records, expected


class GrokFixedAcpTests(unittest.TestCase):
    def test_requests_have_distinct_ids_and_no_authentication_or_model_input(self):
        first = probe.requests(Path("中文 project"), "first")
        second = probe.requests(Path("中文 project"), "second")
        self.assertEqual(tuple(item["method"] for item in first), probe.METHODS)
        self.assertTrue({item["id"] for item in first if "id" in item}.isdisjoint(
            {item["id"] for item in second if "id" in item}))
        self.assertNotIn("id", first[2])
        self.assertEqual(first[2]["params"]["sessionId"], probe.MISSING_SESSION)
        for forbidden in ("session/prompt", "authenticate", "session/request_permission"):
            with self.subTest(method=forbidden), self.assertRaises(ValueError):
                probe.exchange(None, None, None, {"method": forbidden})

    def test_observed_initialize_requires_current_version_and_no_cached_auth(self):
        request = probe.requests(Path("project"), "test")[0]
        response = native_responses()[1]
        response["id"] = request["id"]
        self.assertEqual(probe.validate_response(request, response)["status"], "passed")
        for keys, value in [(('id',), 'old'), (('result', 'protocolVersion'), 2),
                            (('result', 'protocolVersion'), True), (('result', '_meta'), None),
                            (('result', '_meta', 'agentVersion'), '1.0.31'),
                            (('result', '_meta', 'defaultAuthMethodId'), 'cached_token'),
                            (('result', 'authMethods'), [{'id': 'grok.com'}, {'id': 'cached_token'}]),
                            (('result', '_meta', 'mcpServers'), [{'name': 'unexpected'}])]:
            changed = copy.deepcopy(response)
            target = changed
            for key in keys[:-1]:
                target = target[key]
            target[keys[-1]] = value
            with self.subTest(keys=keys), self.assertRaises(ValueError):
                probe.validate_response(request, changed)

    def test_new_session_auth_block_and_missing_history_are_not_successful_sessions(self):
        records, expected = transcript()
        for index, status in [(1, 'blocked_by_auth'), (3, 'missing_session_rejected'), (4, 'missing_session_rejected')]:
            request = expected[index]
            response = next(row['message'] for row in records if row['direction'] == 'stdout' and row['message']['id'] == request['id'])
            self.assertEqual(probe.validate_response(request, response)['status'], status)
            with self.assertRaises(ValueError):
                probe.validate_response(request, {'jsonrpc': '2.0', 'id': request['id'], 'result': {'sessionId': 'unexpected'}})

    def test_observed_transcript_requires_unique_complete_correlated_responses(self):
        records, expected = transcript()
        probe.validate_transcript(records, expected)
        for replacement in [records + [records[-1]], records[:-1], [records[1], records[0], *records[2:]],
                            records + [{'direction': 'stdout', 'message': {'jsonrpc': '2.0', 'id': 'old', 'result': {}}}],
                            records + [{'direction': 'stdout', 'message': {'jsonrpc': '2.0', 'id': None, 'result': {}}}]]:
            with self.subTest(length=len(replacement)), self.assertRaises(ValueError):
                probe.validate_transcript(replacement, expected)

    def test_unexpected_model_or_approval_events_are_rejected(self):
        for message in [{'jsonrpc': '2.0', 'method': 'session/update', 'params': {'update': {'sessionUpdate': 'agent_message_chunk'}}},
                        {'jsonrpc': '2.0', 'method': 'session/request_permission', 'id': 'approval'},
                        {'jsonrpc': '2.0', 'method': '_x.ai/mcp/servers_updated', 'params': {'mcpServers': ['unexpected']}}]:
            with self.subTest(message=message), self.assertRaises(ValueError):
                probe.validate_notification(message)
        probe.validate_notification({'jsonrpc': '2.0', 'method': '_x.ai/mcp/servers_updated', 'params': {'mcpServers': []}})

    def test_private_lock_cannot_reassociate_a_replaced_or_exited_leader(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = Path(temporary) / 'leader.sock'
            leader = mock.Mock()
            leader.process.pid = 42
            leader.process.poll.return_value = None
            endpoint.with_suffix('.lock').write_text('42')
            probe.owned_leader(leader, endpoint)
            endpoint.with_suffix('.lock').write_text('43')
            with self.assertRaises(ValueError):
                probe.owned_leader(leader, endpoint)
            endpoint.with_suffix('.lock').write_text('42')
            leader.process.poll.return_value = 0
            with self.assertRaises(ValueError):
                probe.owned_leader(leader, endpoint)

    def test_real_transport_retains_utf8_and_native_nonzero_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            child = probe.Recorder([sys.executable, '-c', 'import json,sys; print(json.dumps({"text":"中文\\nEnglish"}),flush=True); sys.exit(37)'],
                                   python_fixture_environment(root), root)
            try:
                value = child.messages.get(timeout=5)
                self.assertEqual(value, {'text': '中文\nEnglish'})
                result = probe.wait_idle_exit(child)
                self.assertTrue(result['exited_within_5s'])
                self.assertEqual(result['exit_code_before_cleanup'], 37)
            finally:
                self.assertFalse(child.close())

    def test_real_transport_timeout_cleanup_is_not_reported_as_natural_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            child = probe.Recorder([sys.executable, '-c', 'import sys,time; print("{}",flush=True); time.sleep(60)'],
                                   python_fixture_environment(root), root)
            try:
                self.assertEqual(child.messages.get(timeout=5), {})
                with mock.patch.object(probe, 'EXIT_TIMEOUT', 0.05):
                    result = probe.wait_idle_exit(child)
                self.assertFalse(result['exited_within_5s'])
                self.assertIsNone(result['exit_code_before_cleanup'])
            finally:
                self.assertTrue(child.close())
                self.assertIsNotNone(child.process.poll())

    def test_real_transport_early_exit_does_not_satisfy_a_missing_response(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            child = probe.Recorder([sys.executable, '-c', 'import sys; sys.stdin.readline(); print("fixture early exit",file=sys.stderr,flush=True); sys.exit(37)'],
                                   python_fixture_environment(root), root)
            try:
                with mock.patch.object(probe, 'owned_leader'), self.assertRaisesRegex(ValueError, 'stdio'):
                    probe.exchange(child, mock.Mock(reader_errors=[]), None, probe.requests(root, 'early')[0])
            finally:
                child.close()
            self.assertEqual(child.process.returncode, 37)
            self.assertIn('fixture early exit', [row['message'] for row in child.records if row['direction'] == 'stderr'])

    def test_report_redacts_identity_and_private_paths_but_keeps_protocol_ids(self):
        root = Path('/isolated')
        value = {'id': 'request-id', 'hostname': 'host', 'agentInstanceId': 'instance',
                 'cwd': '/isolated/project', 'access_token': 'fixture-only', 'version': '1.0.30'}
        cleaned = probe.clean(value, root)
        self.assertEqual(cleaned['id'], 'request-id')
        self.assertEqual(cleaned['cwd'], '<isolated-probe>/project')
        for key in ('hostname', 'agentInstanceId', 'access_token'):
            self.assertEqual(cleaned[key], '<redacted>')


if __name__ == '__main__':
    unittest.main()
