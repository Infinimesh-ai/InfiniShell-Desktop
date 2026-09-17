#!/usr/bin/env python3
"""官方运行器离线回归：只用虚构登录文件和本机套接字，不调用 CLI 或模型。"""

import http.client
import contextlib
import hashlib
import io
import os
from pathlib import Path
import socket
import stat
import tempfile
import time
import unittest
from unittest.mock import patch

import run_grok_adapter_live as shared
from grok_adapter_runner_tests import acceptance_fixture
from run_grok_official_adapter_live import (
    MODEL, OfficialTunnel, audit_private_settings, copy_private_auth, main, official_environment,
    prepare_native, public_events, rejected_origin_event,
)


MARKETPLACE_INITIALIZATION = b'''[marketplace]
default_skills_installs_purged = true
official_marketplace_auto_installed = true
[[marketplace.sources]]
name = "xAI Official"
git = "https://github.com/xai-org/plugin-marketplace.git"
'''


class OfficialRunnerTests(unittest.TestCase):
    def settings_before(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / "home/.grok").mkdir(parents=True)
            _, settings = prepare_native(root, Path("/tmp/fixed-grok"), Path("/tmp/auth-source"), 12345)
            return settings.read_bytes()

    def test_exact_native_marketplace_initialization_keeps_byte_difference_visible(self):
        before = self.settings_before()
        audit = audit_private_settings(before, before + MARKETPLACE_INITIALIZATION)
        self.assertFalse(audit["bytes_unchanged"])
        self.assertNotEqual(audit["before_sha256"], audit["after_sha256"])
        self.assertTrue(audit["permission_section_unchanged"])
        self.assertTrue(audit["native_marketplace_initialization_only"])
        self.assertTrue(audit["settings_scope_verified"])
        self.assertEqual(audit["changed_top_level_tables"], ["marketplace"])

    def test_unchanged_and_comment_only_settings_keep_same_permission_scope(self):
        before = self.settings_before()
        for after in (before, before + b"\n# native formatting only\n"):
            audit = audit_private_settings(before, after)
            self.assertTrue(audit["settings_scope_verified"])
            self.assertTrue(audit["permission_section_unchanged"])
            self.assertFalse(audit["native_marketplace_initialization_only"])
        self.assertTrue(audit_private_settings(before, before)["bytes_unchanged"])
        self.assertFalse(audit_private_settings(before, before + b"\n# native formatting only\n")["bytes_unchanged"])

    def test_persistent_permission_grant_is_rejected_even_with_exact_marketplace(self):
        before = self.settings_before()
        grant = b'''[[permission.rules]]
action = "allow"
tool = "write"
'''
        after = before + grant + MARKETPLACE_INITIALIZATION
        audit = audit_private_settings(before, after)
        self.assertFalse(audit["permission_section_unchanged"])
        self.assertFalse(audit["settings_scope_verified"])
        self.assertFalse(audit["native_marketplace_initialization_only"])
        self.assertIn("permission", audit["changed_top_level_tables"])

    def test_changed_permission_policy_or_model_is_not_native_initialization(self):
        before = self.settings_before()
        changes = [before.replace(b'action = "ask"', b'action = "allow"'),
            before.replace(b'grok-4.6-build', b'unapproved-model')]
        for after in changes:
            audit = audit_private_settings(before, after + MARKETPLACE_INITIALIZATION)
            self.assertFalse(audit["settings_scope_verified"])
            self.assertFalse(audit["native_marketplace_initialization_only"])

    def test_marketplace_extra_fields_or_changed_sources_are_rejected(self):
        before = self.settings_before()
        changes = [MARKETPLACE_INITIALIZATION.replace(b'https://github.com/xai-org/plugin-marketplace.git',
                b'https://unknown.example/private-marketplace.git'),
            MARKETPLACE_INITIALIZATION.replace(b'official_marketplace_auto_installed = true',
                b'official_marketplace_auto_installed = false'),
            MARKETPLACE_INITIALIZATION.replace(b'[marketplace]\n', b'[marketplace]\nauto_approve = true\n')]
        for initialization in changes:
            audit = audit_private_settings(before, before + initialization)
            self.assertTrue(audit["permission_section_unchanged"])
            self.assertFalse(audit["settings_scope_verified"])
            self.assertFalse(audit["native_marketplace_initialization_only"])

    def test_settings_comparison_rejects_integer_for_boolean_and_unexpected_table(self):
        before = self.settings_before()
        changes = [before + MARKETPLACE_INITIALIZATION.replace(b'auto_installed = true', b'auto_installed = 1'),
            before.replace(b'support_permission = true', b'support_permission = 1'),
            before + b'[unrecognized]\nvalue = true\n']
        for after in changes:
            self.assertFalse(audit_private_settings(before, after)["settings_scope_verified"])

    def test_invalid_settings_do_not_export_values_or_pass_scope_verification(self):
        before = self.settings_before()
        audit = audit_private_settings(before, b'dummy-private-setting = "unterminated')
        self.assertFalse(audit["toml_parse_succeeded"])
        self.assertFalse(audit["settings_scope_verified"])
        self.assertNotIn("dummy-private-setting", str(audit))
        self.assertEqual(audit["parse_error_type"], "TOMLDecodeError")

    def test_auth_copy_is_opaque_private_and_excludes_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"; source.mkdir(mode=0o700)
            target = root / "target"; target.mkdir(mode=0o700)
            auth = source / "auth.json"
            auth.write_bytes(b"not-real-auth-and-deliberately-not-json")
            auth.chmod(0o600)
            (source / "config.toml").write_text("do not copy")
            result = copy_private_auth(source, target)
            self.assertEqual(result.read_bytes(), auth.read_bytes())
            self.assertEqual(stat.S_IMODE(result.stat().st_mode), 0o600)
            self.assertEqual([item.name for item in target.iterdir()], ["auth.json"])
            self.assertEqual(stat.S_IMODE(auth.stat().st_mode), 0o600)

    def test_auth_symlink_shared_mode_and_existing_destination_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"; source.mkdir()
            target = root / "target"; target.mkdir()
            auth = source / "auth.json"
            auth.write_bytes(b"dummy"); auth.chmod(0o644)
            with self.assertRaises(ValueError):
                copy_private_auth(source, target)
            auth.chmod(0o600)
            (target / "auth.json").write_bytes(b"existing")
            with self.assertRaises(FileExistsError):
                copy_private_auth(source, target)
            (target / "auth.json").unlink()
            auth.rename(source / "elsewhere")
            auth.symlink_to(source / "elsewhere")
            with self.assertRaises(OSError):
                copy_private_auth(source, target)

    def test_environment_does_not_inherit_api_keys_proxies_or_custom_endpoints(self):
        with patch.dict(os.environ, {"ANTHROPIC_API_KEY": "dummy", "ANTHROPIC_AUTH_TOKEN": "dummy",
                "ANTHROPIC_BASE_URL": "https://invalid.example", "XAI_API_KEY": "dummy",
                "HTTPS_PROXY": "https://invalid.example", "GROK_PRODUCTION_CLI_CHAT_PROXY_BASE_URL": "invalid"}):
            environment = official_environment(Path("/tmp/private-root"), 12345)
        self.assertFalse(any(key.startswith("ANTHROPIC") for key in environment))
        for key in ("XAI_API_KEY", "INFINISHELL_GROK_BYOK_KEY", "GROK_PRODUCTION_CLI_CHAT_PROXY_BASE_URL"):
            self.assertNotIn(key, environment)
        self.assertEqual(environment["HTTPS_PROXY"], "http://127.0.0.1:12345")
        self.assertEqual(environment["GROK_DISABLE_API_KEY_AUTH"], "1")

    def test_official_entrypoint_rejects_api_environment_argument(self):
        arguments = ["runner", "--test-binary", "test", "--grok", "grok", "--supervisor", "supervisor",
            "--official-grok-home", "private-home", "--output", "evidence.ndjson",
            "--api-environment-file", "must-not-read.json"]
        with patch("sys.argv", arguments), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as result:
                main()
        self.assertEqual(result.exception.code, 2)

    def test_configuration_keeps_native_official_model_without_custom_backend(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / "home/.grok").mkdir(parents=True)
            wrapper, settings = prepare_native(root, Path("/tmp/fixed-grok"), Path("/tmp/auth-source"), 12345)
            configuration = settings.read_text()
            self.assertIn(f'default = "{MODEL}"', configuration)
            for forbidden in ("base_url", "env_key", "api_backend", "parity-claude", "bypass"):
                self.assertNotIn(forbidden, configuration)
            compile(wrapper.read_text(), str(wrapper), "exec")
            self.assertIn("'agent','stdio','--leader-socket'", wrapper.read_text())

    def test_nonofficial_host_port_and_http_proxy_are_rejected_without_network(self):
        tunnel = OfficialTunnel(30); port = tunnel.start()
        try:
            with patch.object(tunnel, "connect", side_effect=AssertionError("不得发起网络请求")):
                for target in ("evil.example:443", "cli-chat-proxy.grok.com:80",
                        "cli-chat-proxy.grok.com.evil.example:443", "127.0.0.1:443"):
                    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                    connection.request("CONNECT", target)
                    self.assertEqual(connection.getresponse().status, 403)
                    connection.close()
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                connection.request("GET", "https://cli-chat-proxy.grok.com/v1/models")
                self.assertEqual(connection.getresponse().status, 501)
                connection.close()
            self.assertEqual(tunnel.forwarded, 0)
            self.assertEqual(len(tunnel.events), 4)
            self.assertTrue(all(event["event"] == "rejected_nonofficial_origin" for event in tunnel.events))
            self.assertNotIn("evil.example", str(tunnel.events))
        finally:
            self.assertTrue(tunnel.close())

    def test_rejected_origin_diagnostics_classify_fixed_families_without_exporting_authority(self):
        cases = {"api.x.ai:443": "xai_api", "github.com:443": "github_asset_or_marketplace",
            "o123.ingest.sentry.io:443": "sentry_ingest", "o123.ingest.us.sentry.io:443": "sentry_ingest",
            "dummy-secret.private.example:443": "unknown", "api.x.ai:80": "unknown",
            "api.x.ai.evil.example:443": "unknown", "user:dummy-secret@api.x.ai:443": "unknown"}
        for authority, kind in cases.items():
            event = rejected_origin_event(authority)
            self.assertEqual(event["origin_kind"], kind)
            self.assertEqual(event["authority_sha256"], hashlib.sha256(authority.encode()).hexdigest())
            self.assertNotIn(authority, str(event))
            self.assertNotIn("dummy-secret", str(event))

    def test_official_tls_is_opaque_and_shutdown_drains_active_sockets(self):
        tunnel = OfficialTunnel(30); port = tunnel.start()
        upstream, server = socket.socketpair()
        client = socket.create_connection(("127.0.0.1", port), timeout=2)
        server.settimeout(2)
        try:
            with patch.object(tunnel, "connect", return_value=upstream):
                client.sendall(b"CONNECT cli-chat-proxy.grok.com:443 HTTP/1.0\r\n\r\n")
                response = b""
                while b"\r\n\r\n" not in response:
                    response += client.recv(1024)
                self.assertIn(b"200 Connection Established", response)
                client.sendall(b"opaque-dummy-tls")
                self.assertEqual(server.recv(128), b"opaque-dummy-tls")
                server.sendall(b"opaque-reply")
                self.assertEqual(client.recv(128), b"opaque-reply")
                self.assertTrue(tunnel.close())
            self.assertNotIn("opaque", str(tunnel.events))
            self.assertEqual(tunnel.forwarded, 1)
            self.assertFalse(tunnel.connections)
        finally:
            client.close(); server.close(); upstream.close()

    def test_expired_deadline_and_connection_budget_block_before_connect(self):
        for expired in (True, False):
            tunnel = OfficialTunnel(30); port = tunnel.start()
            if expired:
                tunnel.deadline = time.monotonic() - 1
            else:
                tunnel.forwarded = 32
            try:
                with patch.object(tunnel, "connect", side_effect=AssertionError("预算外不能联网")):
                    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                    connection.request("CONNECT", "auth.x.ai:443")
                    self.assertEqual(connection.getresponse().status, 403)
                    connection.close()
            finally:
                self.assertTrue(tunnel.close())

    def test_dns_cannot_redirect_official_connection_to_private_address(self):
        address = [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", 443))]
        with patch("socket.getaddrinfo", return_value=address), patch("socket.socket") as factory:
            with self.assertRaises(OSError):
                OfficialTunnel.connect("auth.x.ai")
            factory.assert_not_called()

    def test_acceptance_keeps_official_and_api_labels_mutually_exclusive(self):
        events = acceptance_fixture()
        output = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        self.assertTrue(shared.verified_acceptance(0, output, events))
        self.assertFalse(shared.verified_acceptance(0, output, events, official=True))
        events[0]["official_grok_model_tested"] = True
        self.assertTrue(shared.verified_acceptance(0, output, events, official=True))
        self.assertFalse(shared.verified_acceptance(0, output, events))
        events[-1]["cleanup_confirmed"] = False
        self.assertFalse(shared.verified_acceptance(0, output, events, official=True))

    def test_public_evidence_never_exports_unrecognized_native_text(self):
        events = [{"event": "acceptance_failed", "reason": "dummy-secret", "output": "dummy-secret",
            "details": {"token": "dummy-secret"}, "exit_code": 1},
            {"event": "turn_finished", "output": "PARITY_ONE", "outcome": "Completed"}]
        clean = public_events(events)
        self.assertNotIn("dummy-secret", str(clean))
        self.assertEqual(clean[1]["output"], "PARITY_ONE")
        self.assertEqual(clean[0]["exit_code"], 1)


if __name__ == "__main__":
    unittest.main()
