#!/usr/bin/env python3
"""Grok 1.0.41 零输入探针的离线协议与边界回归。"""

import json
import unittest

import run_grok_1041_zero_input_p0 as probe


class ZeroInputP0Tests(unittest.TestCase):
    def test_request_allowlist_never_writes_a_model_prompt(self):
        for method in ("session/prompt", "session/cancel", "tools/call", "session/load"):
            with self.subTest(method=method), self.assertRaises(probe.ProbeRejected):
                probe.request(1, method, {})
        self.assertEqual(probe.request(1, "initialize", {})["method"], "initialize")

    def test_malformed_frames_fail_before_projection(self):
        for raw in (b'{"jsonrpc":"2.0","id":1,"id":2}\n',
                    b'{"jsonrpc":"2.0","value":NaN}\n',
                    b'{"jsonrpc":"1.0"}\n',
                    b'{"jsonrpc":"2.0"}'):
            with self.subTest(raw=raw), self.assertRaises(probe.ProbeRejected):
                probe.decode_frame(raw)

    def test_initialize_requires_exact_native_version_and_cached_auth(self):
        base = {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
            "_meta": {"agentVersion": "1.0.41"}, "authMethods": [{"id": "cached_token"}],
            "agentCapabilities": {"loadSession": True}}}
        self.assertTrue(probe.response_summary("initialize", base)["cached_auth_advertised"])
        for changed in ({"_meta": {"agentVersion": "1.0.40"}},
                        {"authMethods": [{"id": "grok.com"}]},
                        {"_meta": None}):
            frame = json.loads(json.dumps(base))
            frame["result"].update(changed)
            with self.subTest(changed=changed), self.assertRaises(probe.ProbeRejected):
                probe.response_summary("initialize", frame)

    def test_session_requires_native_uuid_and_no_rpc_error(self):
        valid = {"jsonrpc": "2.0", "id": 3,
            "result": {"sessionId": "00000000-0000-4000-8000-000000000001"}}
        self.assertTrue(probe.response_summary("session/new", valid)["new_session_accepted"])
        with self.assertRaises(probe.ProbeRejected):
            probe.response_summary("session/new", {"result": {"sessionId": "not-a-session"}})
        with self.assertRaises(probe.ProbeRejected):
            probe.response_summary("session/new", {"error": {"code": -32000}})


if __name__ == "__main__":
    unittest.main()
