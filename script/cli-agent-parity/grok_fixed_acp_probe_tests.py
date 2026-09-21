#!/usr/bin/env python3
"""固定 Grok ACP 边界的离线回归；真实 Python 子进程只验证 I/O 与清理，不冒充 Grok。"""

import copy
import json
from pathlib import Path, PurePosixPath, PureWindowsPath
import sys
import tempfile
import unittest
from unittest import mock

import grok_fixed_acp_probe as probe
from probe_claude_no_credentials_tests import python_fixture_environment


def native_responses(version=probe.VERSION):
    if version not in probe.VERSION_RELEASES:
        raise ValueError("测试版本不在固定输入清单")
    fixture = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/grok-1.0.30-handshake.ndjson"
    rows = [json.loads(line) for line in fixture.read_text(encoding="utf-8").splitlines()]
    responses = {row["message"]["id"]: copy.deepcopy(row["message"]) for row in rows
                 if row["direction"] == "stdout" and "id" in row["message"]}
    # 只替换本测试所核对的原生身份；其余历史响应保持原始字节语义，不作为 1.0.34 实录。
    responses[1]["result"]["_meta"]["agentVersion"] = version
    return responses


def transcript(version=probe.VERSION):
    expected = probe.requests(Path("isolated-project"), "current-probe")
    responses = native_responses(version)
    records = []
    native_id = 0
    for request in expected:
        records.append({"direction": "stdin", "message": request})
        if "id" in request:
            native_id += 1
            if version == probe.VERSION and request["method"] == "session/new":
                session_id = "019d0000-0000-7000-8000-000000000040"
                for phase in probe.SETUP_PHASES:
                    records.append({"direction": "stdout", "message": {"jsonrpc": "2.0",
                        "method": probe.SETUP_METHOD, "params": {"method": "session/new", "phase": phase,
                            "sessionId": None if phase in probe.SETUP_PHASES[:5] else session_id}}})
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

    def test_historical_initialize_requires_explicit_legacy_version(self):
        request = probe.requests(Path("project"), "legacy")[0]
        response = native_responses(probe.LEGACY_VERSION)[1]
        response["id"] = request["id"]
        self.assertEqual(
            probe.validate_response(request, response, probe.LEGACY_VERSION)["agent_version"],
            probe.LEGACY_VERSION,
        )
        with self.assertRaises(ValueError):
            probe.validate_response(request, response)

    def test_new_session_auth_block_and_missing_history_are_not_successful_sessions(self):
        records, expected = transcript()
        for index, status in [(1, 'blocked_by_auth'), (3, 'missing_session_rejected'), (4, 'missing_session_rejected')]:
            request = expected[index]
            response = next(row['message'] for row in records if row['direction'] == 'stdout'
                            and row['message'].get('id') == request['id'])
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

    def test_current_setup_notifications_require_exact_order_and_stable_session(self):
        records, expected = transcript()
        probe.validate_transcript(records, expected)
        setup_indexes = [index for index, row in enumerate(records)
                         if row["message"].get("method") == probe.SETUP_METHOD]

        reordered = copy.deepcopy(records)
        left, right = setup_indexes[1:3]
        reordered[left], reordered[right] = reordered[right], reordered[left]
        with self.assertRaisesRegex(ValueError, "乱序"):
            probe.validate_transcript(reordered, expected)

        duplicate = copy.deepcopy(records)
        duplicate.insert(setup_indexes[-1], copy.deepcopy(duplicate[setup_indexes[-1]]))
        with self.assertRaisesRegex(ValueError, "重复"):
            probe.validate_transcript(duplicate, expected)

        changed = copy.deepcopy(records)
        changed[setup_indexes[-1]]["message"]["params"]["phase"] = "old_phase"
        with self.assertRaisesRegex(ValueError, "阶段"):
            probe.validate_transcript(changed, expected)

    def test_historical_transcript_rejects_current_setup_notification(self):
        records, expected = transcript(probe.P0_VERSION)
        probe.validate_transcript(records, expected, probe.P0_VERSION)
        records.insert(2, {"direction": "stdout", "message": {"jsonrpc": "2.0",
            "method": probe.SETUP_METHOD, "params": {"method": "session/new", "phase": "auth",
                "sessionId": None}}})
        with self.assertRaisesRegex(ValueError, "当前固定版本"):
            probe.validate_transcript(records, expected, probe.P0_VERSION)

    def test_unexpected_model_or_approval_events_are_rejected(self):
        for message in [{'jsonrpc': '2.0', 'method': 'session/update', 'params': {'update': {'sessionUpdate': 'agent_message_chunk'}}},
                        {'jsonrpc': '2.0', 'method': 'session/request_permission', 'id': 'approval'},
                        {'jsonrpc': '2.0', 'method': '_x.ai/mcp/servers_updated', 'params': {'mcpServers': ['unexpected']}}]:
            with self.subTest(message=message), self.assertRaises(ValueError):
                probe.validate_notification(message)
        probe.validate_notification({'jsonrpc': '2.0', 'method': '_x.ai/mcp/servers_updated', 'params': {'mcpServers': []}})

    def test_setup_notification_rejects_unknown_fields_method_and_identity(self):
        session_id = "019d0000-0000-7000-8000-000000000040"
        base = {"jsonrpc": "2.0", "method": probe.SETUP_METHOD,
                "params": {"method": "session/new", "phase": "persistence_init", "sessionId": session_id}}
        probe.validate_notification(base)
        changes = [
            ("phase", "old_phase"),
            ("method", "session/load"),
            ("sessionId", None),
            ("sessionId", "not-a-session"),
        ]
        for key, value in changes:
            changed = copy.deepcopy(base)
            changed["params"][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                probe.validate_notification(changed)
        extra = copy.deepcopy(base)
        extra["params"]["legacy"] = True
        with self.assertRaises(ValueError):
            probe.validate_notification(extra)

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

    def test_mapped_pid_is_bounded_read_only_and_rejects_invalid_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            lock = Path(temporary) / 'leader.lock'
            lock.write_bytes(b'42\n')
            before = lock.stat()
            self.assertEqual(probe.read_leader_pid(lock, mapped=True), 42)
            self.assertEqual(lock.read_bytes(), b'42\n')
            self.assertEqual(lock.stat().st_mtime_ns, before.st_mtime_ns)
            for value in (b'', b'0', b'-42', b'42\x0043', b'\xff', b'4294967296', b'4' * 33):
                lock.write_bytes(value)
                with self.subTest(value=value), self.assertRaises(ValueError):
                    probe.read_leader_pid(lock, mapped=True)
                self.assertEqual(lock.read_bytes(), value)

    def test_lock_read_failure_keeps_operation_and_native_error_without_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            lock = Path(temporary) / 'leader.lock'
            lock.write_bytes(b'42')
            for mapped in (False, True):
                failure = PermissionError(13, 'fixture denied')
                with self.subTest(mapped=mapped), mock.patch.object(Path, 'open', side_effect=failure):
                    with self.assertRaises(PermissionError) as caught:
                        probe.read_leader_pid(lock, mapped=mapped)
                self.assertIs(caught.exception, failure)
                self.assertEqual(failure.errno, 13)
                self.assertEqual(failure.probe_operation, 'leader_lock_pid_read')
                self.assertEqual(failure.probe_file_category, 'private_leader_lock')
                self.assertEqual(failure.probe_read_mode, 'read_only_mmap' if mapped else 'regular_read')

    def test_startup_empty_pid_waits_for_same_live_leader_within_original_deadline(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = Path(temporary) / 'leader.sock'
            endpoint.touch()
            lock = endpoint.with_suffix('.lock')
            lock.touch()
            leader = mock.Mock()
            leader.process.pid = 42
            leader.process.poll.return_value = None
            clock = [0.0]

            def complete_pid(delay):
                clock[0] += delay
                lock.write_bytes(b'42')

            with mock.patch.object(probe.time, 'monotonic', side_effect=lambda: clock[0]), \
                    mock.patch.object(probe.time, 'sleep', side_effect=complete_pid) as sleep:
                probe.wait_leader(leader, endpoint)
            sleep.assert_called_once_with(0.05)
            self.assertEqual(lock.read_bytes(), b'42')
            self.assertIsNone(leader.process.poll())

    def test_startup_empty_pid_times_out_without_becoming_an_accepted_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = Path(temporary) / 'leader.sock'
            endpoint.touch()
            lock = endpoint.with_suffix('.lock')
            lock.touch()
            leader = mock.Mock()
            leader.process.pid = 42
            leader.process.poll.return_value = None
            clock = [0.0]

            def advance(delay):
                clock[0] += delay

            with mock.patch.object(probe, 'LEADER_TIMEOUT', 0.1), \
                    mock.patch.object(probe.time, 'monotonic', side_effect=lambda: clock[0]), \
                    mock.patch.object(probe.time, 'sleep', side_effect=advance) as sleep:
                with self.assertRaisesRegex(ValueError, '期限'):
                    probe.wait_leader(leader, endpoint)
            self.assertEqual(sleep.call_count, 2)
            self.assertEqual(clock[0], 0.1)
            self.assertEqual(lock.read_bytes(), b'')
            with self.assertRaises(probe.EmptyLeaderPid):
                probe.owned_leader(leader, endpoint)

    def test_startup_nonempty_wrong_pid_or_nonregular_file_is_rejected_without_retry(self):
        for value in (b'43', b'invalid', b'4' * 33, None):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as temporary:
                endpoint = Path(temporary) / 'leader.sock'
                lock = endpoint.with_suffix('.lock')
                if value is None:
                    lock.mkdir()
                else:
                    lock.write_bytes(value)
                leader = mock.Mock()
                leader.process.pid = 42
                leader.process.poll.return_value = None
                with mock.patch.object(probe.time, 'sleep') as sleep, self.assertRaises(ValueError):
                    probe.wait_leader(leader, endpoint)
                sleep.assert_not_called()

    def test_startup_empty_pid_cannot_hide_the_owned_leader_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = Path(temporary) / 'leader.sock'
            endpoint.with_suffix('.lock').touch()
            leader = mock.Mock()
            leader.process.pid = 42
            leader.process.poll.return_value = None

            def exit_leader(delay):
                self.assertEqual(delay, 0.05)
                leader.process.poll.return_value = 37

            with mock.patch.object(probe.time, 'sleep', side_effect=exit_leader) as sleep:
                with self.assertRaisesRegex(ValueError, '退出'):
                    probe.wait_leader(leader, endpoint)
            sleep.assert_called_once_with(0.05)

    @unittest.skipUnless(sys.platform == 'win32', '真实 LockFileEx 跨进程契约仅在 Windows 运行')
    def test_windows_exclusive_lock_allows_only_mapped_pid_from_owned_live_child(self):
        # 使用原生字节范围排他锁复现 fs2 行为；不冒充 Grok 的 ACP 或认证接口。
        source = r'''
import ctypes, json, msvcrt, os, sys
from ctypes import wintypes
class OVERLAPPED(ctypes.Structure):
    _fields_ = [('Internal', ctypes.c_size_t), ('InternalHigh', ctypes.c_size_t),
                ('Offset', wintypes.DWORD), ('OffsetHigh', wintypes.DWORD), ('hEvent', wintypes.HANDLE)]
kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.LockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                             wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(OVERLAPPED)]
kernel.LockFileEx.restype = wintypes.BOOL
with open(sys.argv[1], 'w+b', buffering=0) as lock:
    lock.write(str(os.getpid()).encode('ascii'))
    state = OVERLAPPED()
    if not kernel.LockFileEx(msvcrt.get_osfhandle(lock.fileno()), 3, 0, 0xffffffff, 0xffffffff, ctypes.byref(state)):
        raise ctypes.WinError(ctypes.get_last_error())
    print(json.dumps({'pid': os.getpid(), 'exclusive_lock': True}), flush=True)
    sys.stdin.readline()
'''
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            endpoint = root / 'leader.sock'
            lock = endpoint.with_suffix('.lock')
            child = probe.Recorder([sys.executable, '-c', source, str(lock)],
                                   python_fixture_environment(root), root)
            try:
                ready = child.messages.get(timeout=5)
                self.assertEqual(ready, {'pid': child.process.pid, 'exclusive_lock': True})
                with self.assertRaises(PermissionError):
                    lock.read_bytes()
                before = lock.stat()
                self.assertEqual(probe.read_leader_pid(lock), child.process.pid)
                probe.owned_leader(child, endpoint)
                self.assertEqual(lock.stat().st_mtime_ns, before.st_mtime_ns)
                result = child.finish_eof()
                self.assertTrue(result['stdin_eof_exited_within_5s'])
                self.assertEqual(result['exit_code_before_cleanup'], 0)
                self.assertEqual(lock.read_text(), str(child.process.pid))
                with self.assertRaises(ValueError):
                    probe.owned_leader(child, endpoint)
            finally:
                child.close()
            self.assertIsNotNone(child.process.poll())

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
        # 原生请求始终用 str(project)；不能把 Windows Path 与手写 POSIX 路径混用。
        for root, suffix in [(Path('/isolated'), '\\project' if sys.platform == 'win32' else '/project'),
                             (PurePosixPath('/isolated'), '/project'),
                             (PureWindowsPath(r'C:\isolated'), r'\project')]:
            with self.subTest(root=root):
                value = {'id': 'request-id', 'hostname': 'host', 'agentInstanceId': 'instance',
                         'cwd': str(root / 'project'), 'access_token': 'fixture-only', 'version': '1.0.30'}
                cleaned = probe.clean(value, root)
                self.assertEqual(cleaned['id'], 'request-id')
                self.assertEqual(cleaned['cwd'], '<isolated-probe>' + suffix)
                for key in ('hostname', 'agentInstanceId', 'access_token'):
                    self.assertEqual(cleaned[key], '<redacted>')


if __name__ == '__main__':
    unittest.main()
